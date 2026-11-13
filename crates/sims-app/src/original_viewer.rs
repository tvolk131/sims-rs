//! Iced host for the reusable original-house session.
use super::support::{
    Primitive, atomic_save, render_headless_element, room_theme, scene_cursor, validate_output,
};
use super::{Args, dialog_view};
use anyhow::{Context, Result, ensure};
use iced::{
    Color, Element, Fill, Size, Task,
    widget::{self, button, column, container, progress_bar, row, scrollable, slider, text},
    window,
};
use serde::Deserialize;
use sims_content::Index;
use sims_render::{
    Scene,
    camera_device::Device as CameraDevice,
    dialog::client::{Client as DialogClient, Inputs as DialogInputs, buttons::Key as ButtonKey},
    original_house::OriginalHouse,
};
use sims_runtime::original_session::{
    Options, Progress, Saved, Session, camera_command::Change, clock,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
#[path = "original_hud.rs"]
mod hud;
#[path = "original_viewer/hud_cluster_publication.rs"]
mod hud_cluster_publication;
#[path = "original_viewer/ui_publication.rs"]
mod ui_publication;
#[path = "original_viewer/owned_worker.rs"]
mod owned_worker;
#[path = "original_viewer/worker_host.rs"]
mod worker_host;
#[cfg(test)]
#[path = "original_viewer/player_timing.rs"]
mod player_timing;
#[path = "original_viewer_play.rs"]
mod play;
#[path = "original_viewer/widget_lifetimes.rs"]
mod widget_lifetimes;
#[path = "original_viewer/neighborhood_usability.rs"]
mod neighborhood_usability;
#[path = "original_viewer/viewport_drag.rs"]
mod viewport_drag;
#[path = "original_viewer/resident_shortcuts.rs"]
mod resident_shortcuts;
#[path = "original_viewer/resident_identity.rs"]
mod resident_identity;
#[path = "original_viewer/resident_portrait.rs"]
mod resident_portrait;
#[path = "original_viewer/window_observation.rs"]
mod window_observation;
#[path = "original_viewer/person_panel.rs"]
mod person_panel;
#[path = "original_viewer/game_options.rs"]
mod game_options;

///The replacement application deliberately omits the Windows Interface
///receiver (camera_device::bind). In that configuration its sidebar reads the
///actual household balance through the recovered locale/cap formatter. Do not
///mutate retained native widget text, fabricate DA delivery, or republish the
///current tool merely to redraw a replacement label. When the native receiver
///is hosted, preserve its published/deferred text semantics.
fn household_funds_label(s: &Session, published: String) -> Result<String> {
    let registry = &s
        .host
        .presentation
        .notifications
        .as_ref()
        .context("household notification registry absent")?
        .dispatch
        .registry;
    let mut next = registry.head;
    while let Some(id) = next {
        let node = &registry.nodes[&id];
        if node.handler == sims_vm::commander::house::Receiver::Interface {
            return Ok(published);
        }
        next = node.next;
    }
    let source = s.panel_text()?;
    Ok(sims_formats::strings::decode(
        &source.funds,
        source.language,
    ))
}
fn household_clock_label(session: &Session) -> Result<String> {
    let source = session.panel_text()?;
    Ok(sims_formats::strings::decode(&source.clock, source.language))
}

//485a2e subtracts signed-short grab offsets before inverse projection.
fn move_ground(session: &Session, pixel: [f32; 2]) -> Result<[i32; 2]> {
    let offset = session
        .host
        .presentation
        .move_tool
        .as_ref()
        .context("Move offsets absent")?
        .tool
        .tool
        .position;
    let q = [
        (pixel[0] as i16).wrapping_sub(offset[1] as i16) as f32,
        (pixel[1] as i16).wrapping_sub(offset[0] as i16) as f32,
    ];
    let lot = session
        .simulation
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("Move floor absent")?;
    Ok(OriginalHouse::native_camera(session)?
        .pick_original_ground_in(lot, sims_render::original_lot_size(&session.simulation)?, q)?
        .unwrap_or([-16, -16]))
}

fn buy_catalog_view(
    catalog: &sims_render::camera_device::panels::catalog::Catalog,
    ready: bool,
) -> Element<'_, Message> {
    use sims_vm::interface::buy::input::Event;
    let message = |event| Message::BuyNavigate(catalog.revision(), event);
    let mut view = column![].spacing(8);
    if let Some(v) = catalog.view() {
        if !v.heading.is_empty() {
            view = view.push(text(&v.heading).size(18));
        }
        if catalog.insufficient_funds() {
            view = view.push(text("Not enough household funds.").size(12));
        }
        view = view.push(
            text("Click to place furniture. Drag to rotate; Shift repeats; Esc cancels.").size(12),
        );
        if v.subcatalog != 0 {
            view = view
                .push(button("Categories").on_press_maybe(ready.then_some(message(Event::Back))));
        }
        for (i, (label, state)) in v.labels.iter().zip(v.categories).enumerate() {
            if state.visible {
                view = view.push(
                    button(label.as_str())
                        .on_press_maybe(
                            (ready && state.enabled).then_some(message(Event::Category(i as u8))),
                        )
                        .style(if state.selected {
                            button::primary
                        } else {
                            button::secondary
                        }),
                );
            }
        }
        for &index in &v.products {
            if let Some(product) = catalog.product(index) {
                view = view.push(
                    button(column![
                        text(product.display_name()),
                        text(format!("${}", product.fields.price)).size(12)
                    ])
                    .padding(8)
                    .width(Fill)
                    .style(if catalog.selected(index) {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press_maybe(
                        ready.then_some(Message::BuyProduct(catalog.revision(), index)),
                    ),
                );
            }
        }
        if v.subcatalog != 0 {
            view = view.push(text(format!("Page {}", v.page.wrapping_add(1))).size(12));
        }
        if v.previous || v.next {
            view = view.push(
                row![
                    button("Previous")
                        .on_press_maybe((ready && v.previous).then_some(message(Event::Previous))),
                    button("Next")
                        .on_press_maybe((ready && v.next).then_some(message(Event::Next))),
                ]
                .spacing(8),
            );
        }
    }
    view.into()
}

// Native object IDs can be reused after death/deletion. The imported family
// roster is historical metadata, so an ID alone cannot identify a live Sim.
fn live_resident(session: &Session, id: i16) -> Option<&sims_content::saves::Resident> {
    resident_identity::live(
        &session.simulation.vm.world,
        &session.house.household.as_ref()?.residents,
        id,
    )
}

fn resident_object(session: &Session, resident: &sims_content::saves::Resident) -> Option<i16> {
    resident_identity::object(&session.simulation.vm.world, resident)
}

fn selection_view(
    session: &Session,
    zoom: f32,
    floor: i32,
    native: bool,
) -> Result<sims_sim::pose::portrait::view::Input> {
    if native {
        return session.retained_camera_input();
    }
    let camera = OriginalHouse::camera(
        session,
        1000,
        740,
        session.simulation.vm.world.camera_rotation,
        zoom,
        floor,
    );
    let size = session
        .simulation
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("original selection lot absent")?
        .size as i32;
    Ok(DialogInputs::camera(camera, size)?.view)
}

fn portrait_inputs(session: &Session, zoom: f32, floor: i32, native: bool) -> Result<DialogInputs> {
    let camera = if native {
        OriginalHouse::native_camera(session)?
    } else {
        OriginalHouse::camera(
            session,
            1000,
            740,
            session.simulation.vm.world.camera_rotation,
            zoom,
            floor,
        )
    };
    let size = session
        .simulation
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("portrait lot absent")?
        .size as i32;
    DialogInputs::camera(camera, size)
}

struct Ready {
    restore_index: Option<Index>,
    session: Mutex<Option<Session>>,
    client: Mutex<Option<DialogClient>>,
    device: Mutex<Option<CameraDevice>>,
    icons: Arc<dialog_view::Icons>,
}
fn bind_neighborhood_observation(session: &mut Session, args: &Args) -> Result<()> {
    ensure!(
        args.neighborhood_observation.is_none()
            || session.host.presentation.neighborhood_replacement.is_none(),
        "capture request conflicts with retained neighborhood replacement"
    );
    if session.host.presentation.neighborhood_input.is_some() {
        return Ok(());
    }
    match (&args.neighborhood_summary, &args.neighborhood_observation) {
        (Some(summary), Some(log)) => {
            let captured =
                sims_runtime::original_session::neighborhood_input::Captured::from_polling(
                    &session.house,
                    &std::fs::read(summary)?,
                    &std::fs::read(log)?,
                    args.neighborhood_sample,
                )?;
            session.bind_neighborhood_capture(captured)?;
        }
        (None, None) => ensure!(
            args.neighborhood_sample == 0,
            "neighborhood sample has no observation"
        ),
        _ => anyhow::bail!("neighborhood observation requires its evidence summary"),
    }
    Ok(())
}
impl std::fmt::Debug for Ready {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OriginalHouseReady")
    }
}
impl Ready {
    fn load(args: &Args) -> Result<Self> {
        Self::load_inner(
            args,
            #[cfg(test)]
            None,
        )
    }

    fn load_inner(
        args: &Args,
        #[cfg(test)] mut stages: Option<&mut player_timing::LoaderStages>,
    ) -> Result<Self> {
        let result = (|| {
            #[cfg(test)]
            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::IndexScanRoute);
            let index = Index::scan(&args.root)?;
            if args.load.is_some() {
                #[cfg(test)]
                player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::IconsReadyWrapCleanup);
                // Cold player restore must not activate the original starting house.
                // The managed session supplies its entire mutable namespace.
                return Ok(Self {
                    icons: Arc::new(dialog_view::Icons::load(&index)?),
                    restore_index: Some(index),
                    session: Mutex::new(None),
                    client: Mutex::new(None),
                    device: Mutex::new(None),
                });
            }
            #[cfg(test)]
            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::SessionLoad);
            let mut session = Session::load(
                &index,
                args.original
                    .as_deref()
                    .context("original house path absent")?,
                Options {
                    native_postload: args.load.is_none(),
                    native_load_completion: args.load.is_none(),
                    routing: if args.inferred_routing {
                        sims_sim::lot_route::path::Policy::InferredRectangles
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                },
            )?;
            #[cfg(test)]
            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::CameraOpenCompletion);
            let mut device = if args.load.is_none() {
                Some(CameraDevice::open_for_household(
                    &mut session,
                    1000,
                    740,
                    0,
                    sims_runtime::original_session::completion::Config {
                        controller_suppressed: 0,
                        mode: 0,
                    },
                )?)
            } else {
                None
            };
            #[cfg(test)]
            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::NeighborhoodBinding);
            neighborhood_usability::bind_policy(&mut session, args, true)?;
            bind_neighborhood_observation(&mut session, args)?;
            // Original loading and F6 return before ordinary household/dialog work.
            #[cfg(test)]
            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::DialogsResidentOpening);
            let mut client = if args.load.is_none() {
                let mut client = DialogClient::new(&mut session)?;
                let inputs = portrait_inputs(&session, 1., 1, true)?;
                client.bind_resident_buttons(&mut session, inputs)?;
                for _ in 0..10000 {
                    if !client.ui_lookup_pending() {
                        break;
                    }
                    client.advance_ui_lookup(&mut session, 256)?;
                }
                ensure!(
                    !client.ui_lookup_pending(),
                    "resident button startup budget exceeded"
                );
                device
                    .as_mut()
                    .context("opening selection device absent")?
                    .publish_opening_household(&mut session, 0)?;
                device
                    .as_mut()
                    .unwrap()
                    .publish_live_interface(&mut session)?;
                client.bind_live_panel(&session)?;
                client.bind_live_panel_texts(person_panel::load_texts(&index,
                    session.program.text_language.context("live panel language absent")?)?)?;
                client.follow_resident_portraits()?;
                Some(client)
            } else {
                None
            };
            #[cfg(test)]
            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::ClockTransportBuyPreview);
            if args.load.is_none() {
                session.bind_clock(1)?;
                let controls = &mut session
                    .host
                    .presentation
                    .transport
                    .as_mut()
                    .unwrap()
                    .controls;
                // Explicit fresh frontend input, like4ce81c's imported-speed setter.
                controls
                    .set_speed(controls.speed)
                    .map_err(anyhow::Error::msg)?;
                if args.buy_preview {
                    play::bind(
                        &mut session,
                        device.as_mut().unwrap(),
                        client.as_mut().unwrap(),
                    )?;
                }
            }
            #[cfg(test)]
            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::IconsReadyWrapCleanup);
            Ok(Self {
                restore_index: None,
                client: Mutex::new(client),
                session: Mutex::new(Some(session)),
                device: Mutex::new(device),
                icons: Arc::new(dialog_view::Icons::load(&index)?),
            })
        })();
        #[cfg(test)]
        if let Some(stages) = stages {
            stages.finish(result.is_ok());
        }
        result
    }
}

#[derive(Debug, Clone)]
enum Message {
    Diagnostics,
    Options,
    GameOptions,
    GameOptionsSection(u64, sims_render::camera_device::panels::options::Section),
    GameOptionsSave(u64),
    RequestApplicationChoice(neighborhood_usability::Choice),
    CancelApplicationChoice(neighborhood_usability::Confirmation),
    ConfirmApplicationChoice(neighborhood_usability::Confirmation),
    ConfiguredRestart(neighborhood_usability::Choice),
    Buy,
    BuyProduct(u64, usize),
    Sell(u32), //retained cursor lifetime, not a reusable script ID
    ExpandPanel,
    BuyNavigate(u64, sims_vm::interface::buy::input::Event),
    DialogKey(u32),
    Loaded(Result<Arc<Ready>, String>),
    Frame(Instant),
    DisplaySize(Size),
    Pan([i32; 2]),
    Scroll([i32; 2]),
    Toggle,
    Live,
    Needs,
    Personality(sims_render::dialog::client::live_panel::Identity),
    Relationships(sims_render::dialog::client::live_panel::Identity),
    Job(sims_render::dialog::client::live_panel::Identity),
    PersonDetails(sims_render::dialog::client::live_panel::Identity, sims_render::dialog::client::live_panel::Detail),
    ClosePersonDetails(sims_render::dialog::client::live_panel::Identity, sims_render::dialog::client::live_panel::Popup),
    MotiveDetails(usize),
    CloseMotiveDetails,
    Step,
    Speed(f32),
    NativeSpeed(i32),
    Floor(i32),
    Walls(u32),
    Rotate,
    RotateBack,
    ZoomIn,
    ZoomOut,
    Zoom(f32),
    Select(Option<i16>),
    ViewportClick([f32; 2]),
    ViewportEnter,
    ViewportButton {
        down: bool,
        pixel: [f32; 2],
        picked: Option<i16>,
    },
    ViewportRightButton {
        down: bool,
        pixel: Option<[f32; 2]>,
    },
    Shift(bool),
    FocusLost,
    ViewportLeave,
    ViewportMotion {
        pixel: Option<[f32; 2]>,
        pan: [i32; 2],
    },
    GroundChoice(usize),
    GroundCancel,
    ObjectChoice(usize),
    ObjectCancel,
    Resident(i16),
    Queue(i16, usize),
    Cancel(u64),
    Dialog(u64, u32),
    Save,
    Load,
    Restart,
}
///Replacement input queue. Consecutive mouse moves coalesce, but never
///across an enter/leave transition or an executing engine call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum MoveInput {
    Enter,
    Leave,
    Motion([f32; 2]),
    Button {
        kind: sims_runtime::original_session::move_tool::button::Kind,
        pixel: [f32; 2],
        picked: i16,
        shift: bool,
    },
}
#[cfg(test)]
#[derive(Default, Clone, Copy, serde::Serialize)]
struct FrameProfile {
    engine_services_ms: f64,
    draw_preparation_ms: f64,
    presentation_refresh_ms: f64,
    refresh: RefreshProfile,
    engine: EngineProfile,
}
#[cfg(test)]
#[derive(Default, Clone, Copy, serde::Serialize)]
struct EngineProfile {
    ui_lookup: StageProfile,
    ui_wait: StageProfile,
    replacement_people: StageProfile,
    publication: StageProfile,
    tool_selection: StageProfile,
    move_tool: StageProfile,
    move_button: StageProfile,
    transport: StageProfile,
    panel: StageProfile,
    interface: StageProfile,
    camera: StageProfile,
    notification: StageProfile,
    republish_interface: StageProfile,
}
// The expression and its error handling stay at their original call site.
// Test instrumentation observes completed calls, including any synchronous
// child work, without changing budgets, service order, or production branches.
macro_rules! profiled_engine {
    ($profile:expr, $stage:ident, $call:expr) => {{
        let _span = super::support::window_session::span(
            concat!("engine_", stringify!($stage)), || serde_json::Value::Null,
        );
        #[cfg(test)]
        let started = $profile.as_ref().map(|_| Instant::now());
        let result = $call;
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut $profile, started) {
            profile.engine.$stage.record(start);
        }
        result
    }};
}
#[cfg(test)]
#[derive(Default, Clone, Copy, serde::Serialize)]
struct StageProfile {
    calls: u64,
    ms: f64,
    max_ms: f64,
}
#[cfg(test)]
impl StageProfile {
    fn record(&mut self, start: Instant) {
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        self.calls += 1;
        self.ms += elapsed;
        self.max_ms = self.max_ms.max(elapsed);
    }
}
#[cfg(test)]
#[derive(Default, Clone, Copy, serde::Serialize)]
struct RefreshProfile {
    catalogue: StageProfile,
    dialog_collect: StageProfile,
    live_panel: StageProfile,
    dialog_preview: StageProfile,
    normal_frame: StageProfile,
    icons_portraits_residents: StageProfile,
    device_scene: StageProfile,
    display_resize: StageProfile,
    fallback_scene: StageProfile,
}
struct Viewer {
    #[cfg(test)]
    frame_profile: Option<FrameProfile>,
    /// Opt-in research observer; absent in the player application.
    exploration_ticks: Option<Vec<sims_runtime::exploration::Observation>>,
    diagnostics: bool,
    /// Replacement application menu; never published as native Options mode.
    options_open: bool,
    application: neighborhood_usability::Snapshot,
    application_confirmation: Option<neighborhood_usability::Confirmation>,
    application_revision: u64,
    display_size: [u32; 2],
    /// Ephemeral Iced button-press lifetime; absent from saved state.
    widget_epoch: u64,
    thumbnails: Option<hud::Thumbnails>,
    tutorial_highlight: Option<i32>,
    args: Args,
    session: Option<Session>,
    client: Option<DialogClient>,
    device: Option<CameraDevice>,
    icons: Option<Arc<dialog_view::Icons>>,
    art: Option<Arc<dialog_view::HudArt>>,
    hud_cluster: Option<Arc<hud::cluster::Published>>,
    renderer: OriginalHouse,
    scene: Option<Arc<Scene>>,
    resident: i16,
    selected: Option<i16>,
    floor: i32,
    zoom: f32,
    playing: bool,
    speed: f32,
    last: Option<Instant>,
    /// Rebased on cold restore; absolute milliseconds retain sub-frame tails.
    clock_origin: Option<(Instant, u32)>,
    accumulator: f32,
    pan: [i32; 2],
    viewport_drag: viewport_drag::Drag,
    shift: bool,
    viewport_enter_requested: bool,
    move_inputs: std::collections::VecDeque<MoveInput>,
    status: String,
    error: Option<String>,
    choices: Vec<sims_sim::interaction::MenuChoice>,
    menu_error: Option<String>,
    window_dialog_answer: Option<serde_json::Value>,
    window_dialog_answers: u64,
}
impl Viewer {
    fn new(args: Args) -> (Self, Task<Message>) {
        Self::new_inner(
            args,
            #[cfg(test)]
            None,
        )
    }
    #[cfg(test)]
    fn new_with_loader_probe(
        args: Args,
        probe: player_timing::LoaderProbe,
    ) -> (Self, Task<Message>) {
        Self::new_inner(args, Some(probe))
    }
    fn new_inner(
        args: Args,
        #[cfg(test)] probe: Option<player_timing::LoaderProbe>,
    ) -> (Self, Task<Message>) {
        let work = args.clone();
        let task = Task::perform(
            async move {
                #[cfg(test)]
                if let Some(probe) = probe {
                    return probe
                        .measure_staged(|stages| Ready::load_inner(&work, Some(stages)))
                        .map(Arc::new)
                        .map_err(|error| format!("{error:#}"));
                }
                Ready::load(&work)
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"))
            },
            Message::Loaded,
        );
        (
            Self {
                #[cfg(test)]
                frame_profile: None,
                exploration_ticks: None,
                diagnostics: false,
                options_open: false,
                application: neighborhood_usability::Snapshot::loading(&args),
                application_confirmation: None,
                application_revision: 0,
                display_size: [1320, 900],
                widget_epoch: 0,
                thumbnails: None,
                tutorial_highlight: None,
                floor: args.floor,
                playing: !args.paused && args.screenshot.is_none(),
                args,
                session: None,
                client: None,
                device: None,
                icons: None,
                art: None,
                hud_cluster: None,
                renderer: OriginalHouse::default(),
                scene: None,
                resident: 0,
                selected: None,
                zoom: 0.55,
                speed: 1.,
                last: None,
                clock_origin: None,
                accumulator: 0.,
                pan: [0, 0],
                viewport_drag: Default::default(),
                shift: false,
                viewport_enter_requested: false,
                move_inputs: Default::default(),
                status: "Loading original household…".into(),
                error: None,
                choices: vec![],
                menu_error: None,
                window_dialog_answer: None,
                window_dialog_answers: 0,
            },
            task,
        )
    }
    fn fail(&mut self, e: impl std::fmt::Display) {
        self.viewport_drag.reset();
        self.error = Some(e.to_string());
        self.playing = false;
        self.accumulator = 0.;
    }
    fn install(&mut self, ready: Arc<Ready>) -> Result<()> {
        self.window_dialog_answer = None;
        self.viewport_drag.reset();
        self.thumbnails = None;
        self.tutorial_highlight = None;
        self.session = ready.session.lock().unwrap().take();
        if let Some(session) = &mut self.session {
            window_observation::enable(&mut session.simulation);
        }
        self.client = ready.client.lock().unwrap().take();
        self.device = ready.device.lock().unwrap().take();
        self.art = Some(Arc::new(ready.icons.publish_art()));
        self.icons = Some(ready.icons.clone());
        self.renderer = OriginalHouse::default();
        self.scene = None;
        self.error = None;
        self.selected = None;
        self.last = None;
        self.clock_origin = None;
        if let Some(path) = self.args.load.clone() {
            if let Some(index) = &ready.restore_index {
                self.load_with_index(&path, index)?;
            } else {
                self.load(&path)?;
            }
        } else {
            let session = self
                .session
                .as_ref()
                .context("loaded session already consumed")?;
            self.resident = session
                .house
                .household
                .as_ref()
                .context("household absent")?
                .residents
                .get(self.args.resident as usize - 1)
                .context("resident index is outside this household")?
                .object as i16;
            if let Some(device) = &mut self.device {
                let s = self.session.as_mut().unwrap();
                if device.controls().floor != self.args.floor {
                    device.begin(s, Change::Floor(self.args.floor))?;
                    let mut complete = false;
                    for _ in 0..10000 {
                        if device.advance(s, 256)? {
                            complete = true;
                            break;
                        }
                    }
                    ensure!(complete, "initial camera work budget exceeded");
                }
                self.floor = device.controls().floor;
                self.zoom = (1u32 << device.controls().zoom) as f32 / 8.;
            }
            self.select_resident(self.resident)?;
        }
        ensure!(
            self.args.at_tick <= 1_000_000,
            "initial advance exceeds limit"
        );
        for _ in 0..self.args.at_tick {
            // CLI fast-forward is an explicit diagnostic pass count. It does
            // not spend elapsed-frame credit or invent application wall time.
            while !self.step_with_clock(false)? {
                if !self.client.as_ref().unwrap().pending() {
                    break;
                }
            }
            if self
                .session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_some()
            {
                break;
            }
        }
        if self.args.load.is_none()
            && self.native_transport()
            && !self.playing
            && !self.presentation_pending()
            && self
                .session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_none()
        {
            self.session.as_mut().unwrap().begin_pause(true)?;
        }
        self.application = neighborhood_usability::Snapshot::capture(
            self.session
                .as_ref()
                .context("installed household absent")?,
            &self.args,
        )?;
        self.status = if self.client.as_ref().is_some_and(|c| c.pending()) {
            "Preparing dialog…"
        } else if self
            .session
            .as_ref()
            .is_some_and(|s| s.simulation.vm.world.dialogs.modal.is_some())
        {
            "Waiting for your response."
        } else {
            "Original household loaded. Select a Sim and an object to queue an action."
        }
        .into();
        self.prepare_thumbnail_catalog()?;
        self.refresh()
    }
    fn select_resident(&mut self, id: i16) -> Result<()> {
        let s = self.session.as_mut().context("household still loading")?;
        ensure!(
            live_resident(s, id).is_some(),
            "resident is no longer present"
        );
        ensure!(
            s.simulation.has_interaction_actor(id),
            "resident has no live action queue"
        );
        let view = selection_view(s, self.zoom, self.floor, self.device.is_some())?;
        s.begin_selection(Some(id))?;
        let mut complete = false;
        for _ in 0..10_000 {
            if s.house_notification_pending() {
                self.device
                    .as_mut()
                    .context("selection notification device absent")?
                    .advance_notification_with_client(
                        s,
                        self.client.as_mut().context("notification client absent")?,
                        256,
                    )?;
                continue;
            }
            if s.advance_selection(view, 256)? {
                complete = true;
                break;
            }
        }
        ensure!(complete, "native selection service budget exceeded");
        self.resident = id;
        Ok(())
    }
    /// Explicit application resource stage, separate from pure frame refresh.
    /// Retain incomplete native asset callers and resume this cursor on updates.
    fn prepare_and_publish(&mut self) -> Result<()> {
        let _span = super::support::window_session::span("prepare_publication", || serde_json::Value::Null);
        #[cfg(test)]
        let start = self.frame_profile.as_ref().map(|_| Instant::now());
        if !self.client.as_ref().is_some_and(|client| client.pending())
            && !self
                .device
                .as_ref()
                .is_some_and(CameraDevice::normal_view_frame_pending)
        {
            if let Some(session) = &mut self.session {
                if session.person_draw_preparation_ready() {
                    if !session.person_draw_preparation_pending() {
                        let objects = session.person_draw_objects()?;
                        session.begin_person_draw_preparation(objects)?;
                    }
                    while !session.advance_person_draw_preparation(16)? {}
                }
            }
        }
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, start) {
            profile.draw_preparation_ms += start.elapsed().as_secs_f64() * 1000.;
        }
        self.refresh()
    }
    fn refresh(&mut self) -> Result<()> {
        let _span = super::support::window_session::span("publication_refresh", || serde_json::Value::Null);
        #[cfg(test)]
        let start = self.frame_profile.as_ref().map(|_| Instant::now());
        let result = self.refresh_inner();
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, start) {
            profile.presentation_refresh_ms += start.elapsed().as_secs_f64() * 1000.;
        }
        self.publish_hud_cluster();
        result
    }
    fn refresh_inner(&mut self) -> Result<()> {
        if self.art.is_none() {
            self.art = self
                .icons
                .as_ref()
                .map(|icons| Arc::new(icons.publish_art()));
        }
        #[cfg(test)]
        let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
        self.prepare_catalog_thumbnails()?;
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
            profile.refresh.catalogue.record(start);
        }
        let footer = self.hud_height();
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        let client = self.client.as_mut().context("dialog client absent")?;
        if let Some(device) = &mut self.device {
            if device.normal_view_frame_pending() {
                device.prepare_view_frame(session)?;
            }
        }
        if session.ui_portrait_pending() || session.player_input_blocks_ticks() {
            // A newly opened viewer has no cached Iced handles. These are the
            // already-published widgets/images; rebuilding them must not wait
            // for the suspended getter or redraw its partial world state.
            #[cfg(test)]
            let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
            client.collect(session)?;
            #[cfg(test)]
            if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
                profile.refresh.dialog_collect.record(start);
            }
            #[cfg(test)]
            let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
            let icons = self.icons.as_ref().context("dialog icons absent")?;
            let prepared = (|| -> Result<()> {
                icons.prepare(&session.simulation.vm.world.dialogs.records)?;
                icons.prepare_portraits(
                    &session.simulation.vm.world.dialogs.records,
                    client.images(),
                )?;
                icons.prepare_residents(client.buttons(), client.images())?;
                Ok(())
            })();
            self.art = Some(Arc::new(icons.publish_art()));
            prepared?;
            #[cfg(test)]
            if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
                profile.refresh.icons_portraits_residents.record(start);
            }
        }
        if !client.pending() && !session.clock_pending() && !session.simulation.selection_work_pending() {
            #[cfg(test)]
            let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
            client.refresh_live_panel(session)?;
            #[cfg(test)]
            if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
                profile.refresh.live_panel.record(start);
            }
        }
        if self.device.is_some()
            && (session.camera_command_pending()
                || session.interface_update_pending()
                || session.control_panel_pending()
                || session.ui_portrait_pending()
                || session.ui_wait_pending()
                || session.house_notification_pending()
                || session.simulation.selection_work_pending()
                || session.player_input_blocks_ticks())
        {
            if self.scene.is_none() {
                self.scene = Some(self.device.as_ref().unwrap().scene().clone());
            }
            return Ok(());
        }
        #[cfg(test)]
        let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
        client.collect(session)?;
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
            profile.refresh.dialog_collect.record(start);
        }
        if client.pending() && self.scene.is_some() {
            return Ok(());
        }
        #[cfg(test)]
        let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
        let preview = if client.pending() {
            client.preview(session)?
        } else {
            None
        };
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
            profile.refresh.dialog_preview.record(start);
        }
        if preview.is_none()
            && self.device.is_some()
            && !session.clock_pending()
            && session.simulation.vm.scheduled_object_visit.is_none()
        {
            #[cfg(test)]
            let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
            self.device.as_mut().unwrap().prepare_view_frame(session)?;
            #[cfg(test)]
            if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
                profile.refresh.normal_frame.record(start);
            }
        }
        let shown = preview.as_ref().unwrap_or(session);
        if self
            .selected
            .is_some_and(|id| !shown.simulation.vm.world.objects.contains_key(&id))
        {
            self.selected = None;
            self.choices.clear();
        }
        #[cfg(test)]
        let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
        let icons = self.icons.as_ref().context("dialog icons absent")?;
        let prepared = (|| -> Result<()> {
            icons.prepare(&shown.simulation.vm.world.dialogs.records)?;
            icons.prepare_portraits(&shown.simulation.vm.world.dialogs.records, client.images())?;
            icons.prepare_residents(client.buttons(), client.images())?;
            Ok(())
        })();
        self.art = Some(Arc::new(icons.publish_art()));
        prepared?;
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
            profile.refresh.icons_portraits_residents.record(start);
        }
        if let Some(device) = &mut self.device {
            if preview.is_none() {
                #[cfg(test)]
                let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
                device.refresh(shown)?;
                #[cfg(test)]
                if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
                    profile.refresh.device_scene.record(start);
                }
                if !self.diagnostics && !session.clock_pending() {
                    let height = self.display_size[1].saturating_sub(footer).max(1);
                    if [device.scene().width, device.scene().height]
                        != [self.display_size[0], height]
                    {
                        #[cfg(test)]
                        let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
                        device.resize_display(session, self.display_size, height)?;
                        #[cfg(test)]
                        if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start)
                        {
                            profile.refresh.display_resize.record(start);
                        }
                    }
                }
                self.scene = Some(device.scene().clone());
                self.floor = device.controls().floor;
                self.zoom = (1u32 << device.controls().zoom) as f32 / 8.;
                return Ok(());
            }
        }
        let camera = OriginalHouse::camera(
            shown,
            1000,
            740,
            shown.simulation.vm.world.camera_rotation,
            self.zoom,
            self.floor,
        );
        #[cfg(test)]
        let stage_start = self.frame_profile.as_ref().map(|_| Instant::now());
        self.scene = Some(Arc::new(self.renderer.scene(shown, camera)?));
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, stage_start) {
            profile.refresh.fallback_scene.record(start);
        }
        Ok(())
    }
    /// True only when the lot clock advances. A suspended constructor consumes
    /// no speed-control time and receives no intervening gameplay commands.
    fn step(&mut self) -> Result<bool> {
        self.step_with_clock(true)
    }
    fn step_with_clock(&mut self, elapsed: bool) -> Result<bool> {
        let _span = super::support::window_session::span("engine_step", || {
            serde_json::json!({"elapsed":elapsed,
                "tick":self.session.as_ref().map(|session| session.simulation.vm.tick),
                "clock_pending":self.session.as_ref().is_some_and(Session::clock_pending)})
        });
        let before = self.session.as_ref().map(|s| s.simulation.vm.tick);
        #[cfg(test)]
        let start = self.frame_profile.as_ref().map(|_| Instant::now());
        let result = self.step_with_clock_inner(elapsed);
        #[cfg(test)]
        if let (Some(profile), Some(start)) = (&mut self.frame_profile, start) {
            profile.engine_services_ms += start.elapsed().as_secs_f64() * 1000.;
        }
        if result.is_ok()
            && self.exploration_ticks.is_some()
            && self.session.as_ref().map(|s| s.simulation.vm.tick) != before
        {
            let observation = exploration::tick_observation(self)?;
            self.exploration_ticks.as_mut().unwrap().push(observation);
        }
        result
    }
    fn step_with_clock_inner(&mut self, elapsed: bool) -> Result<bool> {
        if self
            .device
            .as_ref()
            .is_some_and(CameraDevice::normal_view_frame_pending)
        {
            return Ok(false);
        }
        if self.client.as_ref().is_some_and(|c| c.ui_lookup_pending()) {
            let client = self.client.as_mut().unwrap();
            let session = self.session.as_mut().context("household still loading")?;
            for _ in 0..16 {
                if profiled_engine!(
                    self.frame_profile,
                    ui_lookup,
                    client.advance_ui_lookup(session, 256)
                )? {
                    break;
                }
            }
            self.status = if client.ui_lookup_pending() {
                "Updating resident portraits…".into()
            } else {
                String::new()
            };
            return Ok(false);
        }
        if self.session.as_ref().is_some_and(|s| s.ui_wait_pending()) {
            profiled_engine!(self.frame_profile, ui_wait, self.poll_application_ui())?;
            return Ok(false);
        }
        if (self.viewport_enter_requested || !self.move_inputs.is_empty())
            && !self.presentation_children_pending()
            && !self.session.as_ref().is_some_and(Session::clock_pending)
        {
            self.flush_enter();
            let input = self.move_inputs.pop_front().unwrap();
            if self.current_move() {
                let session = self.session.as_mut().context("household still loading")?;
                match input {
                    MoveInput::Enter => session.begin_move_tool_viewport_enter()?,
                    MoveInput::Leave => {
                        session.begin_move_tool_cleanup(
                            sims_sim::object_catalog::move_tool::cleanup::Kind::ViewportLeave,
                        )?;
                        self.status.clear();
                    }
                    MoveInput::Button {
                        kind,
                        pixel,
                        picked,
                        shift,
                    } => {
                        use sims_runtime::original_session::move_tool::{button, motion};
                        let position = move_ground(session, pixel)?;
                        session.begin_move_button(button::Input {
                            kind,
                            pointer: motion::Input {
                                position,
                                screen: [pixel[1] as i16, pixel[0] as i16],
                                picked,
                            },
                            shift,
                        })?;
                    }
                    MoveInput::Motion(pixel) => {
                        if session
                            .host
                            .presentation
                            .move_tool
                            .as_ref()
                            .is_some_and(|m| {
                                m.tool.held.is_some() && matches!(m.tool.tool.mode, 1 | 2)
                            })
                        {
                            let position = move_ground(session, pixel)?;
                            session.begin_move_tool_motion(
                                sims_runtime::original_session::move_tool::motion::Input {
                                    picked: 0,
                                    position,
                                    screen: [pixel[1] as i16, pixel[0] as i16],
                                },
                            )?;
                        }
                    }
                }
            }
            return Ok(false);
        }
        let session = self.session.as_mut().context("household still loading")?;
        if let Some(outcome) = session.dispatch_menu_command()? {
            self.selected = None;
            self.status = match outcome {
                sims_runtime::original_session::live_menu::Outcome::Cancelled => {
                    "Interaction menu closed."
                }
                sims_runtime::original_session::live_menu::Outcome::Queued {
                    accepted: true,
                    ..
                } => "Interaction queued.",
                sims_runtime::original_session::live_menu::Outcome::Queued {
                    accepted: false,
                    ..
                } => "Interaction could not be queued.",
            }
            .into();
            return Ok(false);
        }
        if let Some(device) = &mut self.device {
            if session
                .host
                .presentation
                .property_refresh
                .as_ref()
                .and_then(|p| p.panel_owner())
                .is_some()
            {
                profiled_engine!(
                    self.frame_profile,
                    replacement_people,
                    device.publish_replacement_people(
                        session,
                        self.client
                            .as_mut()
                            .context("property People client absent")?,
                    )
                )?;
            }
            if session.publication_pending() {
                use sims_runtime::original_session::publication::Progress as Publication;
                if device
                    .panel_widgets()
                    .is_some_and(|w| w.presentation().is_none())
                {
                    // The existing Iced layout displays People until a
                    // replacement panel owner publishes another panel.
                    device.bind_panel_presentation(
                        session,
                        sims_render::camera_device::panels::Panel::People,
                    )?;
                }
                if device.panel_widgets().is_some_and(|w| w.texts().is_none()) {
                    device.bind_panel_text(session)?;
                }
                if device.panel_widgets().is_some_and(|w| w.access().is_none()) {
                    device.bind_panel_access(session)?;
                }
                if device.panel_widgets().is_some_and(|widgets| widgets.options().is_none()) {
                    device.bind_panel_options(session)?;
                }
                if device
                    .panel_widgets()
                    .is_some_and(|w| w.catalog().is_none())
                {
                    device.bind_panel_catalog(session, 8)?;
                }
                for _ in 0..16 {
                    match profiled_engine!(
                        self.frame_profile,
                        publication,
                        device.advance_publication(session, 256)
                    )? {
                        Publication::Complete => {
                            self.status.clear();
                            return Ok(false);
                        }
                        Publication::Pending => (),
                        Publication::Panels(_)
                            if profiled_engine!(
                                self.frame_profile,
                                replacement_people,
                                device.publish_replacement_people(
                                    session,
                                    self.client.as_mut().context("People client absent")?,
                                )
                            )? =>
                        {
                            ()
                        }
                        Publication::Panels(_) => anyhow::bail!(
                            "control panel update needs an unsupported service: {:?}",
                            device.panel_widgets().and_then(|w| w.boundary())
                        ),
                        other => anyhow::bail!(
                            "interface update needs an unsupported service: {other:?}"
                        ),
                    }
                }
                self.status = "Updating controls…".into();
                return Ok(false);
            }
            if session.move_button_pending() {
                for _ in 0..16 {
                    if profiled_engine!(
                        self.frame_profile,
                        move_button,
                        device.advance_move_button(session, 256)
                    )? {
                        break;
                    }
                }
                self.status = if session.move_button_pending() {
                    "Placing furniture…".into()
                } else {
                    match session
                        .host
                        .presentation
                        .move_tool
                        .as_ref()
                        .and_then(|m| m.button_hint())
                    {
                        Some(sims_runtime::original_session::move_tool::button::Hint::Funds) => {
                            "Not enough household funds.".into()
                        }
                        Some(
                            sims_runtime::original_session::move_tool::button::Hint::Placement(_),
                        ) => "Cannot place the object here.".into(),
                        Some(sims_runtime::original_session::move_tool::button::Hint::Spent(
                            cost,
                        )) => format!("Purchased for ${cost}."),
                        None => String::new(),
                    }
                };
                return Ok(false);
            }
            if session.tool_selection_pending() {
                for _ in 0..16 {
                    if profiled_engine!(
                        self.frame_profile,
                        tool_selection,
                        device.advance_tool_selection(session, 256)
                    )? {
                        break;
                    }
                }
                self.status = if session.tool_selection_pending() {
                    "Updating catalog…".into()
                } else {
                    // Finish the synchronous input before the normal dirty
                    // frame reads its fields. This remains pending through
                    // saves and must return before another input or lot pass.
                    profiled_engine!(
                        self.frame_profile,
                        republish_interface,
                        session.republish_interface()
                    )?;
                    String::new()
                };
                return Ok(false);
            }
            if session.move_tool_pending() {
                for _ in 0..16 {
                    if profiled_engine!(
                        self.frame_profile,
                        move_tool,
                        device.advance_move_tool(session, 256)
                    )? {
                        break;
                    }
                }
                self.status = if session.move_tool_pending() {
                    "Preparing selected object…".into()
                } else {
                    String::new()
                };
                return Ok(false);
            }
            if session.transport_pending() {
                for _ in 0..16 {
                    if profiled_engine!(
                        self.frame_profile,
                        transport,
                        device.advance_transport(session, 256)
                    )? {
                        profiled_engine!(
                            self.frame_profile,
                            republish_interface,
                            session.republish_interface()
                        )?;
                        break;
                    }
                }
                self.status = if session.transport_pending() {
                    "Updating pause controls…".into()
                } else {
                    String::new()
                };
                return Ok(false);
            }
            if session.control_panel_pending() {
                for _ in 0..16 {
                    if profiled_engine!(self.frame_profile, panel, device.advance_panel(session))? {
                        break;
                    }
                }
                self.status = "Updating control panel…".into();
                return Ok(false);
            }
            if session.interface_update_pending() {
                for _ in 0..16 {
                    if profiled_engine!(
                        self.frame_profile,
                        interface,
                        device.advance_interface(session, 256)
                    )? {
                        break;
                    }
                }
                self.status = "Updating interface…".into();
                return Ok(false);
            }
            if session.camera_command_pending() {
                for _ in 0..16 {
                    if profiled_engine!(self.frame_profile, camera, device.advance(session, 256))? {
                        // The setter has returned. Publish its shared CPState
                        // dirty bits before accepting another input, even when
                        // the replacement viewer is paused. This uses4ce390's
                        // existing world/panel children and saved ownership.
                        profiled_engine!(
                            self.frame_profile,
                            republish_interface,
                            session.republish_interface()
                        )?;
                        break;
                    }
                }
                self.status = "Updating view…".into();
                return Ok(false);
            }
            if session.house_notification_pending() {
                for _ in 0..16 {
                    if profiled_engine!(
                        self.frame_profile,
                        notification,
                        device.advance_notification_with_client(
                            session,
                            self.client.as_mut().context("notification client absent")?,
                            256,
                        )
                    )? {
                        break;
                    }
                }
                self.status = "Updating selection…".into();
                return Ok(false);
            }
        }
        if session.object_input_pending() && !session.running_menu() {
            for _ in 0..16 {
                if !session.object_input_pending() {
                    self.selected = None;
                    self.status.clear();
                    break;
                }
                if session
                    .host
                    .presentation
                    .object_input
                    .as_ref()
                    .unwrap()
                    .ready()
                {
                    if session.host.presentation.clock.is_some() {
                        session.begin_running_menu()?;
                    }
                    self.status = "Choose an interaction.".into();
                    break;
                }
                session.advance_object_input(256)?;
                self.status = "Preparing interaction…".into();
            }
            return Ok(false);
        }
        if session.ground_input_pending() && !session.running_menu() {
            let camera = session.retained_camera_input()?;
            for _ in 0..16 {
                if session.house_notification_pending() {
                    self.device
                        .as_mut()
                        .context("ground notification device absent")?
                        .advance_notification_with_client(
                            session,
                            self.client.as_mut().context("notification client absent")?,
                            256,
                        )?;
                    continue;
                }
                let input = session.host.presentation.ground_input.as_ref().unwrap();
                if input.ready() {
                    if session.host.presentation.clock.is_some() {
                        session.begin_running_menu()?;
                    }
                    self.status = "Choose an interaction.".into();
                    break;
                }
                if input.complete() {
                    session.finish_ground_input()?;
                    self.status = "Movement command closed.".into();
                    break;
                }
                session.advance_ground_input(camera, 256)?;
                self.status = "Preparing movement…".into();
            }
            return Ok(false);
        }
        if session.simulation.native_selection.is_some() {
            let view = selection_view(session, self.zoom, self.floor, self.device.is_some())?;
            for _ in 0..16 {
                if session.house_notification_pending() {
                    self.device
                        .as_mut()
                        .context("selection notification device absent")?
                        .advance_notification_with_client(
                            session,
                            self.client.as_mut().context("notification client absent")?,
                            256,
                        )?;
                    continue;
                }
                if session.advance_selection(view, 256)? {
                    break;
                }
            }
            if let Some(operation) = &session.simulation.native_selection {
                self.status = if operation.is_refresh() {
                    "Updating view…"
                } else {
                    "Selecting Sim…"
                }
                .into();
                return Ok(false);
            }
            if let Some(id) = session
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
            {
                self.resident = id;
            }
        }
        let client = self.client.as_mut().context("dialog client absent")?;
        for _ in 0..16 {
            if client.pending() {
                let (complete, _) = client.advance(session, 256)?;
                if !complete {
                    self.status = "Preparing dialog…".into();
                    return Ok(false);
                }
            }
            // Audio output is not connected yet. Consume queued mixer commands
            // silently, as in the demo client, before advancing the simulation.
            session.simulation.vm.sounds.clear();
            let progress = if elapsed && session.host.presentation.clock.is_some() {
                if !session.clock_pending() {
                    session.begin_clock_frame()?;
                }
                match session.advance_clock(256)? {
                    clock::Progress::Pending => return Ok(false),
                    clock::Progress::PollInput => {
                        session.acknowledge_clock_input(session.menu_command_pending())?;
                        return Ok(false);
                    }
                    clock::Progress::Complete { .. } => {
                        self.status.clear();
                        return Ok(false);
                    }
                    clock::Progress::Household(progress) => progress,
                }
            } else {
                session.step()?
            };
            match progress {
                Progress::UiEffect(thread) => {
                    let sims_vm::Status::AwaitingUiEffect(call) =
                        &session.simulation.vm.threads[&thread].status
                    else {
                        unreachable!()
                    };
                    let request = call.request();
                    match request.kind {
                        sims_vm::ui_effect::Kind::Control => {
                            use sims_vm::ui_effect::widgets::{Bitmap, Kind, Node, Query};
                            let mut ids = vec![2011, 2012, 2013, 2014, 2023, 2024];
                            if session.host.presentation.control_panel.is_some() {
                                ids.push(2010);
                            }
                            if session.host.presentation.tool_selection.is_some() {
                                ids.push(2008);
                            }
                            if session.host.presentation.tool_selection.as_ref().and_then(|owner| owner.mode_context()).is_some() {
                                ids.push(2009);
                            }
                            if client.live_panel().is_some() {
                                ids.extend(sims_render::dialog::client::live_panel::Category::ALL.map(|category| category.control()));
                            }
                            ensure!(
                                ids.contains(&i32::from(request.value)),
                                "original UI control {} is not bound in this viewer",
                                request.value
                            );
                            let mut nodes = vec![Node {
                                kind: Kind::Window,
                                bitmap: None,
                                children: (1..=ids.len()).collect(),
                            }];
                            nodes.extend(ids.into_iter().map(|id| Node {
                                kind: Kind::Button,
                                bitmap: Some(Bitmap {
                                    handle: id as u32,
                                    resource: id,
                                }),
                                children: vec![],
                            }));
                            let found = sims_vm::ui_effect::widgets::find(
                                &nodes,
                                Some(0),
                                Query::Control(i32::from(request.value)),
                            )
                            .map_err(anyhow::Error::msg)?;
                            if found.is_some() {
                                self.tutorial_highlight =
                                    request.enabled.then_some(i32::from(request.value));
                            }
                        }
                        sims_vm::ui_effect::Kind::Portrait if client.buttons().is_some() => {
                            if let Some(device) = &mut self.device {
                                device.refresh(session)?;
                                self.scene = Some(device.scene().clone());
                            }
                            let inputs = portrait_inputs(
                                session,
                                self.zoom,
                                self.floor,
                                self.device.is_some(),
                            )?;
                            client.begin_portrait_highlight(session, thread, request, inputs)?;
                            if profiled_engine!(
                                self.frame_profile,
                                ui_lookup,
                                client.advance_ui_lookup(session, 256)
                            )? {
                                continue;
                            }
                            return Ok(false);
                        }
                        sims_vm::ui_effect::Kind::Relationship if client.live_panel().is_some() => {
                            client.highlight_live_relationship(session, request)?;
                        }
                        sims_vm::ui_effect::Kind::Portrait
                        | sims_vm::ui_effect::Kind::Relationship => {
                            // Native4256f4 uses neighborhood slots. A missing
                            // neighbor is a proven return without UI changes;
                            // a present one still needs its actual image/panel host.
                            ensure!(
                                sims_vm::ui_effect::neighbor_guid(
                                    &session.simulation.vm.world,
                                    request.value
                                )
                                .map_err(anyhow::Error::msg)?
                                .is_none(),
                                "original {:?} highlight requires its neighborhood image/panel host",
                                request.kind
                            );
                        }
                    }
                    session
                        .simulation
                        .vm
                        .complete_ui_effect(&session.program, thread, request)
                        .map_err(anyhow::Error::msg)?;
                }
                Progress::HouseNotification => {
                    self.device
                        .as_mut()
                        .context("house notification requires its application listener host")?
                        .advance_notification_with_client(
                            session,
                            self.client.as_mut().context("notification client absent")?,
                            256,
                        )?;
                    return Ok(false);
                }
                Progress::DialogConstruction(thread) => {
                    let camera = if self.device.is_some() {
                        OriginalHouse::native_camera(session)?
                    } else {
                        OriginalHouse::camera(
                            session,
                            1000,
                            740,
                            session.simulation.vm.world.camera_rotation,
                            self.zoom,
                            self.floor,
                        )
                    };
                    let size = session
                        .simulation
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .context("original lot absent")?
                        .size as i32;
                    client.begin(session, thread, DialogInputs::camera(camera, size)?)?;
                    // Preserve this stable view while reentrant construction
                    // temporarily creates/poses/removes a person for its portrait.
                    self.scene = Some(Arc::new(self.renderer.scene(session, camera)?));
                }
                Progress::Dialog => {
                    client.collect(session)?;
                    if session.simulation.vm.world.dialogs.modal.is_some() {
                        self.status = "Waiting for your response.".into();
                    }
                    return Ok(false);
                }
                Progress::Tick => {
                    client.collect(session)?;
                    return Ok(true);
                }
                Progress::PartialPass => {
                    client.collect(session)?;
                    // A partial pass consumes replacement scheduling time but
                    // no game time. It must not build a catch-up backlog.
                    self.accumulator = (self.accumulator - 0.04).max(0.);
                    return Ok(false);
                }
            }
        }
        self.status = "Preparing interface…".into();
        Ok(false)
    }
    fn current_move(&self) -> bool {
        self.session.as_ref().is_some_and(|s| {
            s.host
                .presentation
                .current_tool
                .as_ref()
                .and_then(|t| t.current())
                .is_some_and(|id| {
                    matches!(
                        s.host
                            .presentation
                            .tool_selection
                            .as_ref()
                            .and_then(|t| t.receivers().get(&id)),
                        Some(sims_runtime::original_session::tool_selection::Receiver::Move)
                    )
                })
        })
    }
    fn flush_enter(&mut self) {
        if self.viewport_enter_requested {
            self.move_inputs.push_back(MoveInput::Enter);
            self.viewport_enter_requested = false;
        }
    }
    fn presentation_pending(&self) -> bool {
        self.viewport_enter_requested
            || !self.move_inputs.is_empty()
            || self.session.as_ref().is_some_and(Session::clock_pending)
            || self
                .session
                .as_ref()
                .is_some_and(Session::menu_command_pending)
            || self.presentation_children_pending()
    }
    fn people_panel_visible(&self) -> bool {
        use sims_render::camera_device::panels::Panel;
        self.device.as_ref().is_none_or(|device| {
            device.panel_state() != Some(1)
                && device
                    .panel_widgets()
                    .and_then(|widgets| widgets.presentation())
                    .is_none_or(|presentation| presentation.visible(Panel::People))
        })
    }
    fn presentation_children_pending(&self) -> bool {
        self.device
            .as_ref()
            .is_some_and(CameraDevice::normal_view_frame_pending)
            || self.client.as_ref().is_some_and(|c| c.pending())
            || self.session.as_ref().is_some_and(|s| {
                s.normal_view_frame_pending()
                    || s.person_draw_preparation_pending()
                    || s.player_input_blocks_ticks()
                    || s.transport_pending()
                    || s.publication_pending()
                    || s.tool_selection_pending()
                    || s.camera_command_pending()
                    || s.interface_update_pending()
                    || s.control_panel_pending()
                    || s.ui_portrait_pending()
                    || s.ui_wait_pending()
                    || s.house_notification_pending()
                    || s.simulation.selection_work_pending()
                    || s.simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_)))
            })
    }
    fn native_transport(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.host.presentation.transport.is_some())
    }
    fn native_clock(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.host.presentation.clock.is_some())
    }
    fn native_partial_pass(&self) -> Result<bool> {
        let Some(session) = self
            .session
            .as_ref()
            .filter(|session| session.host.presentation.transport.is_some())
        else {
            return Ok(false);
        };
        let globals = &session.simulation.vm.world.globals;
        let word = |index| {
            globals
                .get(&index)
                .copied()
                .with_context(|| format!("native scheduler global {index} absent"))
        };
        Ok(sims_vm::scheduler::Inputs {
            paused: word(17)?,
            mode: word(19)?,
            lot_number: word(10)?,
            tick: session.simulation.vm.tick as u32,
        }
        .partial())
    }
    fn advance_native_frame_work(&mut self) -> Result<()> {
        let _span = super::support::window_session::span("native_frame_work", || serde_json::Value::Null);
        for _ in 0..16 {
            self.step()?;
            if !self.session.as_ref().unwrap().clock_pending()
                || self.presentation_children_pending()
                || self
                    .session
                    .as_ref()
                    .unwrap()
                    .simulation
                    .vm
                    .world
                    .dialogs
                    .modal
                    .is_some()
            {
                break;
            }
        }
        Ok(())
    }
    fn update_clock_time(&mut self, now: Instant) -> Result<()> {
        let partial = self.native_partial_pass()?;
        let Some(s) = self.session.as_mut() else {
            return Ok(());
        };
        let Some(clock) = &s.host.presentation.clock else {
            return Ok(());
        };
        // A local diagnostic STOPPED hold freezes the application epoch. Real
        // native pause keeps polling, and a suspended synchronous caller owns
        // its return even if a modal changed the viewer's playing flag.
        if !self.playing && !partial && !s.clock_pending() {
            self.clock_origin = None;
            return Ok(());
        }
        let (origin, offset) = *self.clock_origin.get_or_insert((now, clock.now));
        s.set_clock_time(
            offset.wrapping_add(now.saturating_duration_since(origin).as_millis() as u32),
        )
    }
    // One poll per replacement application frame, before its lot ticks.
    // The native application4d1901 calls4764fe before46d9d6. Drawing and
    // wall-clock cadence remain the replacement client's scheduling policy.
    fn poll_application_ui(&mut self) -> Result<bool> {
        if self.device.is_none() {
            return Ok(true);
        }
        let session = self.session.as_mut().context("household still loading")?;
        if session.player_input_blocks_ticks() {
            return Ok(false);
        }
        if !session.ui_wait_pending() {
            // Only an actual committed button activation supplies this state.
            // Selection and tutorial highlighting are not button activations.
            let focused = self
                .client
                .as_ref()
                .and_then(|c| c.buttons())
                .and_then(|b| b.focused_resource());
            session.begin_ui_wait(focused)?;
        }
        for _ in 0..16 {
            match session.advance_ui_wait(256)? {
                sims_runtime::original_session::ui_wait::Progress::Complete => return Ok(true),
                sims_runtime::original_session::ui_wait::Progress::Pending => (),
                sims_runtime::original_session::ui_wait::Progress::Observe(condition) => {
                    if let Some(observation) = self
                        .client
                        .as_ref()
                        .and_then(|client| client.buttons())
                        .and_then(|buttons| buttons.observation(&condition))
                    {
                        session.acknowledge_ui_wait(&condition, observation)?;
                        continue;
                    }
                    if let sims_vm::ui_owner::wait::Condition::Control(id) = condition {
                        use sims_vm::interface::view_control::Widget;
                        let kind = match id {
                            2008 => Some(Widget::Buy),
                            2010 => Some(Widget::Live),
                            _ => None,
                        };
                        if let Some((id, state)) = kind.and_then(|k| {
                            self.device
                                .as_ref()?
                                .panel_widgets()
                                .map(|w| (id, w.button(k)))
                        }) {
                            session.acknowledge_ui_wait(
                                &condition,
                                sims_vm::ui_owner::wait::Observation::Control {
                                    image: Some(id),
                                    state: u32::from(state.selected),
                                },
                            )?;
                            continue;
                        }
                    }
                    if let Some(observation) = self
                        .client
                        .as_ref()
                        .and_then(|c| c.live_panel())
                        .and_then(|p| p.observation(&condition))
                    {
                        session.acknowledge_ui_wait(&condition, observation)?;
                        continue;
                    }
                    if let sims_vm::ui_owner::wait::Condition::Portrait(neighbor) = condition {
                        if self.client.as_ref().is_some_and(|c| c.buttons().is_some()) {
                            let inputs = portrait_inputs(
                                session,
                                self.zoom,
                                self.floor,
                                self.device.is_some(),
                            )?;
                            let client = self.client.as_mut().unwrap();
                            client.begin_portrait_observation(session, neighbor, inputs)?;
                            if client.advance_ui_lookup(session, 256)? {
                                continue;
                            }
                            return Ok(false);
                        }
                    }
                    anyhow::bail!("original UI wait requires its widget owner: {condition:?}")
                }
            }
        }
        Ok(false)
    }
    fn set_pan(&mut self, direction: [i32; 2]) {
        // A published popup lets the lot clock run, but still owns player
        // input. Cursor motion can reach the viewport behind its overlay (or
        // while resizing); it must not queue camera input for the popup's
        // return. Keep the runtime's synchronous camera-owner guard intact.
        self.pan = if self
            .session
            .as_ref()
            .is_some_and(Session::player_input_pending)
        {
            [0, 0]
        } else {
            direction
        };
    }
    fn scroll_pixels(&mut self, displacement: [i32; 2]) -> Result<bool> {
        if self
            .session
            .as_ref()
            .is_some_and(Session::player_input_pending)
        {
            // Also discard intent recorded before this input owner opened.
            // Ignoring a camera gesture must not stop the ordinary lot clock.
            self.pan = [0, 0];
            return Ok(true);
        }
        let Some(device) = &mut self.device else {
            return Ok(true);
        };
        let s = self.session.as_mut().context("household still loading")?;
        if s.simulation.vm.world.dialogs.modal.is_some() {
            return Ok(true);
        }
        let p = s.retained_camera_input()?.center;
        device.begin(
            s,
            Change::Scroll([
                p[0].wrapping_add(displacement[0]),
                p[1].wrapping_add(displacement[1]),
            ]),
        )?;
        for _ in 0..16 {
            if device.advance(s, 256)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn change_view(&mut self, change: Change) -> Result<()> {
        self.device
            .as_ref()
            .context("camera device absent")?
            .begin(
                self.session.as_mut().context("household still loading")?,
                change,
            )?;
        self.status = "Updating view…".into();
        Ok(())
    }
    fn select_object(&mut self, target: Option<i16>) -> Result<()> {
        let session = self.session.as_mut().context("household still loading")?;
        if self.device.is_some()
            && let Some(target) = target
        {
            ensure!(
                session
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .and_then(|p| p.camera.as_ref())
                    .is_some_and(|c| c.settings.application_mode == 2),
                "object interactions require Live mode"
            );
            if !session.begin_object_input(target)? {
                self.selected = None;
                return Ok(());
            }
            self.selected = Some(
                session
                    .host
                    .presentation
                    .object_input
                    .as_ref()
                    .unwrap()
                    .target(),
            );
            self.pan = [0, 0];
            self.accumulator = 0.;
            self.status = "Preparing interaction…".into();
        } else {
            session.clear_picker_target()?;
            self.selected = target.map(|id| {
                session
                    .simulation
                    .vm
                    .world
                    .objects
                    .get(&id)
                    .and_then(|o| o.multitile.as_ref())
                    .and_then(|p| p.head)
                    .unwrap_or(id)
            });
        }
        Ok(())
    }
    fn menu(&mut self) {
        self.choices.clear();
        self.menu_error = None;
        if self.device.is_some() || self.presentation_pending() {
            return;
        }
        let (Some(session), Some(target)) = (&self.session, self.selected) else {
            return;
        };
        if live_resident(session, self.resident).is_none() {
            return;
        }
        let entries = super::support::entries(&session.program, &session.simulation, target);
        for (i, entry) in entries.iter().enumerate() {
            if entry.label.is_empty() {
                continue;
            }
            match session
                .simulation
                .interaction_choice(&session.program, self.resident, target, i)
            {
                Ok(choice) => self.choices.push(choice),
                Err(_) => self.menu_error = Some("Some actions cannot be evaluated yet.".into()),
            }
        }
    }
    fn label(&self, id: i16) -> String {
        let Some(s) = &self.session else {
            return format!("Object {id}");
        };
        if let Some(r) = live_resident(s, id) {
            return r.name.clone();
        }
        object_catalog_label(&s.program, &s.simulation.vm.world, id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Object {id}"))
    }
    fn sell_message(&self) -> Option<Message> {
        if self.error.is_some() || !self.current_move() || self.presentation_pending() {
            return None;
        }
        let s = self.session.as_ref()?;
        if s.player_input_pending() || s.simulation.vm.world.dialogs.modal.is_some() {
            return None;
        }
        let selected = self.device.as_ref()?.furniture_selection()?;
        if !selected.sell_allowed {
            return None;
        }
        Some(Message::Sell(
            s.host.presentation.move_tool.as_ref()?.tool.held?,
        ))
    }
    fn save(&mut self) -> Result<()> {
        self.require_player_save_boundary()?;
        self.refresh()?;
        self.require_player_save_boundary()?;
        let session = self.session.as_ref().context("household still loading")?;
        validate_output(&self.args.root, &self.args.save)?;
        if let Some(p) = self
            .args
            .save
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            std::fs::create_dir_all(p)?;
        }
        atomic_save(
            &self.args.save,
            &serde_json::to_vec(
                &serde_json::json!({"session":session.save_managed()?,"dialog_client":self.client.as_ref().context("dialog client absent")?.save_managed(session)?,
            "camera_device":self.device.as_ref().map(|d| d.save(session)).transpose()?,
            "floor":self.floor,"zoom":self.zoom,"resident":self.resident,"selected":self.selected,"tutorial_highlight":self.tutorial_highlight,"viewport_enter_requested":self.viewport_enter_requested,"move_inputs":self.move_inputs}),
            )?,
        )
    }
    fn require_player_save_boundary(&self) -> Result<()> {
        ensure!(
            self.error.is_none(),
            "player Save refused while application error is present: {}",
            self.error.as_deref().unwrap_or_default()
        );
        ensure!(
            self.menu_error.is_none(),
            "player Save refused while menu error is present: {}",
            self.menu_error.as_deref().unwrap_or_default()
        );
        let session = self.session.as_ref().context("household still loading")?;
        ensure!(
            !session
                .simulation
                .vm
                .threads
                .values()
                .any(|thread| matches!(thread.status, sims_vm::Status::Faulted(_))),
            "player Save refused: faulted VM would overwrite the last known-good player save"
        );
        Ok(())
    }
    fn load(&mut self, path: &Path) -> Result<()> {
        let index = self
            .session
            .as_ref()
            .context("household still loading")?
            .index()
            .clone();
        self.load_with_index(path, &index)
    }
    fn load_with_index(&mut self, path: &Path, index: &Index) -> Result<()> {
        self.viewport_drag.reset();
        self.thumbnails = None;
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let saved = Saved::deserialize(&value["session"])?;
        let managed = saved.version == 2;
        ensure!(
            Some(&saved.source) == self.args.original.as_ref(),
            "snapshot belongs to a different original house"
        );
        let mut session = Session::restore(index, saved)?;
        neighborhood_usability::bind_policy(&mut session, &self.args, false)?;
        bind_neighborhood_observation(&mut session, &self.args)?;
        let client = if value.get("dialog_client").is_some_and(|v| !v.is_null()) {
            let saved = Deserialize::deserialize(&value["dialog_client"])?;
            if managed {
                DialogClient::restore_managed(&session, saved)?
            } else {
                DialogClient::restore(&session, saved)?
            }
        } else {
            ensure!(!managed, "managed player save lost its portrait client");
            DialogClient::new(&mut session)?
        };
        let floor = i32::try_from(value["floor"].as_i64().context("saved floor absent")?)?;
        let zoom = value["zoom"].as_f64().context("saved zoom absent")? as f32;
        ensure!(
            (1..=2).contains(&floor) && zoom.is_finite() && (0.15..=1.5).contains(&zoom),
            "invalid saved view"
        );
        let resident = i16::try_from(
            value["resident"]
                .as_i64()
                .context("saved resident absent")?,
        )?;
        let selected: Option<i16> = Deserialize::deserialize(&value["selected"])?;
        let tutorial_highlight: Option<i32> =
            Deserialize::deserialize(&value["tutorial_highlight"])?;
        ensure!(
            tutorial_highlight.is_none_or(|id| [2011, 2012, 2013, 2014, 2023, 2024].contains(&id)
                || (id == 2008 && session.host.presentation.tool_selection.is_some())
                || (id == 2009 && session.host.presentation.tool_selection.as_ref().and_then(|owner| owner.mode_context()).is_some())
                || (id == 2010 && session.host.presentation.control_panel.is_some())
                || ([4500, 4501, 4503, 4504].contains(&id) && client.live_panel().is_some())),
            "unsupported saved tutorial highlight"
        );
        ensure!(
            live_resident(&session, resident).is_some(),
            "saved selected resident is not a household member"
        );
        ensure!(
            selected.is_none_or(|id| session.simulation.vm.world.objects.contains_key(&id)),
            "saved selected object is absent"
        );
        self.device = value
            .get("camera_device")
            .filter(|v| !v.is_null())
            .map(|saved| CameraDevice::restore(&session, Deserialize::deserialize(saved)?))
            .transpose()?;
        self.application = neighborhood_usability::Snapshot::capture(&session, &self.args)?;
        self.window_dialog_answer = None;
        window_observation::enable(&mut session.simulation);
        self.session = Some(session);
        self.client = Some(client);
        self.scene = None;
        self.floor = floor;
        self.zoom = zoom;
        self.resident = resident;
        self.selected = selected;
        self.tutorial_highlight = tutorial_highlight;
        self.viewport_enter_requested = value
            .get("viewport_enter_requested")
            .map(Deserialize::deserialize)
            .transpose()?
            .unwrap_or(false);
        self.move_inputs = value
            .get("move_inputs")
            .map(Deserialize::deserialize)
            .transpose()?
            .unwrap_or_default();
        self.renderer = OriginalHouse::default();
        self.playing = false;
        self.error = None;
        self.last = None;
        self.clock_origin = None;
        self.pan = [0, 0];
        self.accumulator = 0.;
        self.prepare_thumbnail_catalog()?;
        self.publish_hud_cluster();
        Ok(())
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        self.prepare_viewport_drag(&message);
        let task = self.update_inner(message);
        self.synchronize_viewport_drag();
        // An owner observation of the current UI, not native-call completion.
        // This captures ignored/error/early-return messages without changing
        // their original guards, execution order or task effects.
        self.publish_hud_cluster();
        task
    }
    fn update_inner(&mut self, message: Message) -> Task<Message> {
        let message = match message {
            Message::RequestApplicationChoice(choice) => {
                if self.options_open && self.session.is_some() {
                    self.application_revision = self.application_revision.wrapping_add(1);
                    self.application_confirmation = Some(neighborhood_usability::Confirmation {
                        choice,
                        revision: self.application_revision,
                    });
                }
                return Task::none();
            }
            Message::CancelApplicationChoice(confirmation) => {
                if self.application_confirmation == Some(confirmation) {
                    self.application_confirmation = None;
                    self.application_revision = self.application_revision.wrapping_add(1);
                }
                return Task::none();
            }
            Message::ConfirmApplicationChoice(confirmation) => {
                if !self.options_open || self.application_confirmation != Some(confirmation) {
                    return Task::none();
                }
                self.application_confirmation = None;
                self.application_revision = self.application_revision.wrapping_add(1);
                Message::ConfiguredRestart(confirmation.choice)
            }
            other => other,
        };
        if self.application_confirmation.is_some()
            && !matches!(
                message,
                Message::Options
                    | Message::Diagnostics
                    | Message::Load
                    | Message::Restart
                    | Message::ConfiguredRestart(_)
            )
        {
            return Task::none();
        }
        if self.error.is_some()
            && !matches!(
                message,
                Message::Options
                    | Message::Diagnostics
                    | Message::Load
                    | Message::Restart
                    | Message::ConfiguredRestart(_)
                    | Message::Loaded(_)
            )
        {
            return Task::none();
        }
        if let Message::DisplaySize(size) = message {
            if size.width.is_finite() && size.height.is_finite() {
                self.display_size = [
                    size.width.round().clamp(320., 4096.) as u32,
                    size.height.round().clamp(240., 4096.) as u32,
                ];
                if let Err(e) = self.prepare_and_publish() {
                    self.fail(e);
                }
            }
            return Task::none();
        }
        if matches!(message, Message::Options) {
            self.application_confirmation = None;
            self.application_revision = self.application_revision.wrapping_add(1);
            self.options_open = !self.options_open;
            return Task::none();
        }
        if matches!(message, Message::Diagnostics) {
            self.application_confirmation = None;
            self.application_revision = self.application_revision.wrapping_add(1);
            self.options_open = false;
            self.diagnostics = !self.diagnostics;
            return Task::none();
        }
        if person_panel::request(&message).is_some() {
            let available = self.session.as_ref().zip(self.client.as_ref().and_then(|client| client.live_panel()))
                .is_some_and(|(session, panel)| person_panel::available(panel,
                    sims_render::dialog::client::live_panel::selected_identity(session).ok().flatten(), &message,
                    self.error.is_some() || self.options_open || self.presentation_pending()
                        || session.player_input_pending() || session.simulation.vm.world.dialogs.modal.is_some(),
                    self.people_panel_visible() && self.device.as_ref().is_none_or(|device| device.panel_state() != Some(1))));
            if !available { return Task::none(); }
        }
        if matches!(message, Message::GameOptions) {
            let supported = self.session.as_ref().is_some_and(|session|
                session.host.presentation.tool_selection.as_ref().and_then(|owner| owner.mode_context()).is_some()
                    && session.simulation.vm.world.dialogs.modal.is_none() && !session.player_input_pending())
                && self.device.as_ref().and_then(|device| device.panel_widgets())
                    .is_some_and(|widgets| widgets.button(sims_vm::interface::view_control::Widget::Options).enabled);
            if !supported || self.error.is_some() || self.options_open || self.presentation_pending() { return Task::none(); }
        }
        if let Message::GameOptionsSection(revision, _) | Message::GameOptionsSave(revision) = &message {
            let widgets = self.device.as_ref().and_then(|device| device.panel_widgets());
            let visible = widgets.and_then(|widgets| widgets.presentation())
                .is_some_and(|presentation| presentation.visible(sims_render::camera_device::panels::Panel::Options));
            let blocked = self.error.is_some() || self.options_open || self.presentation_pending()
                || self.session.as_ref().is_none_or(|session| session.player_input_pending()
                    || session.simulation.vm.world.dialogs.modal.is_some()
                    || session.host.presentation.transport.as_ref().is_none_or(|transport| transport.controls.mode != 3));
            if !game_options::available(widgets.and_then(|widgets| widgets.options()).and_then(|options| options.view()),
                *revision, visible, blocked) { return Task::none(); }
        }
        if matches!(message, Message::Buy | Message::Live) {
            use sims_vm::interface::view_control::Widget;
            let kind = if matches!(message, Message::Buy) {
                Widget::Buy
            } else {
                Widget::Live
            };
            if self.session.as_ref().is_none_or(|s| {
                s.simulation.vm.world.dialogs.modal.is_some()
                    || (matches!(message, Message::Buy)
                        && s.host.presentation.tool_selection.is_none())
            }) || self
                .device
                .as_ref()
                .and_then(|d| d.panel_widgets())
                .is_some_and(|w| !w.button(kind).enabled)
            {
                return Task::none();
            }
        }
        if matches!(message, Message::BuyNavigate(..) | Message::BuyProduct(..))
            && (self.error.is_some()
                || self.presentation_pending()
                || self
                    .session
                    .as_ref()
                    .is_none_or(|s| s.simulation.vm.world.dialogs.modal.is_some()))
        {
            return Task::none();
        }
        // Hiding a panel removes its widgets, but a previously queued message
        // can still arrive. Apply the same visibility rule to input as to draw.
        // Motive-detail popups have independent script-visible ownership and
        // keep their own close path, even when the bar strip is hidden.
        if !self.people_panel_visible()
            && matches!(
                message,
                Message::Resident(..)
                    | Message::Needs
                    | Message::MotiveDetails(..)
                    | Message::Queue(..)
                    | Message::Cancel(..)
                    | Message::Select(..)
                    | Message::ViewportClick(..)
                    | Message::GroundChoice(..)
                    | Message::GroundCancel
                    | Message::ObjectChoice(..)
                    | Message::ObjectCancel
            )
        {
            return Task::none();
        }
        if self.native_transport()
            && matches!(message, Message::Toggle | Message::NativeSpeed(_))
            && (self.presentation_pending()
                || self
                    .session
                    .as_ref()
                    .unwrap()
                    .simulation
                    .vm
                    .world
                    .dialogs
                    .modal
                    .is_some())
        {
            return Task::none();
        }
        let commanded_resident = match &message {
            Message::Resident(id) => Some(*id),
            Message::Queue(..) | Message::Cancel(..) => Some(self.resident),
            _ => None,
        };
        if commanded_resident.is_some_and(|id| {
            self.session
                .as_ref()
                .is_some_and(|s| live_resident(s, id).is_none())
        }) {
            // A widget event can outlive the resident it was built for. Do not
            // send it to whichever object now owns that native ID.
            return Task::none();
        }
        if let Message::Pan(direction) = message {
            self.set_pan(direction);
            return Task::none();
        }
        if matches!(message, Message::FocusLost) {
            // Iced buttons retain their pressed flag across Unfocused. End
            // that button lifetime before a later release can activate it.
            self.widget_epoch = self.widget_epoch.wrapping_add(1);
            self.pan = [0, 0];
            self.shift = false;
            return Task::none();
        }
        if let Message::Shift(shift) = message {
            self.shift = shift;
            return Task::none();
        }
        if let Message::ViewportRightButton { down, pixel } = message {
            self.pan = [0, 0];
            if down {
                let ready = !self.viewport_drag_blocked() && !self.presentation_pending();
                let selection = self.session.as_ref().and_then(|session| {
                    if session.host.presentation.transport.as_ref()?.controls.mode != 2 {
                        return None;
                    }
                    let object = self.scene.as_ref()?.pick(pixel?)?;
                    live_resident(session, object)?;
                    let handle = session
                        .simulation
                        .vm
                        .world
                        .scheduler_storage
                        .as_ref()?
                        .handle(object)?;
                    Some((object, handle))
                });
                self.viewport_drag.begin(pixel, ready);
                self.viewport_drag.bind_selection(selection);
            } else {
                self.viewport_drag.end(pixel);
                if let Some((object, handle)) = self.viewport_drag.take_selection() {
                    let current = self.session.as_ref().and_then(|session| {
                        session.simulation.vm.world.scheduler_storage.as_ref()?.handle(object)
                    });
                    if current == Some(handle)
                        && self.scene.as_ref().and_then(|scene| scene.pick(pixel?)) == Some(object)
                        && !self.viewport_drag_blocked()
                        && !self.presentation_pending()
                    {
                        return self.update(Message::Resident(object));
                    }
                }
            }
            return Task::none();
        }
        if let Message::ViewportButton {
            down,
            pixel,
            picked,
        } = message
        {
            if self.current_move() {
                self.flush_enter();
                self.move_inputs.push_back(MoveInput::Button {
                    kind: if down {
                        sims_runtime::original_session::move_tool::button::Kind::Down
                    } else {
                        sims_runtime::original_session::move_tool::button::Kind::Up
                    },
                    pixel,
                    picked: picked.unwrap_or(0),
                    shift: self.shift,
                });
                return Task::none();
            }
            return if down {
                self.update(picked.map_or(Message::ViewportClick(pixel), |id| {
                    Message::Select(Some(id))
                }))
            } else {
                Task::none()
            };
        }
        if matches!(
            message,
            Message::ViewportEnter | Message::ViewportLeave | Message::ViewportMotion { .. }
        ) {
            match message {
                Message::ViewportEnter => {
                    if self.current_move() {
                        self.flush_enter();
                        self.viewport_enter_requested = true;
                    }
                }
                Message::ViewportLeave => {
                    self.pan = [0, 0];
                    if self.current_move() {
                        self.flush_enter();
                        self.move_inputs.push_back(MoveInput::Leave);
                    }
                }
                Message::ViewportMotion { pixel, pan } => {
                    if self.viewport_drag.suppresses_pan() {
                        self.pan = [0, 0];
                        self.viewport_drag.motion(pixel);
                    } else {
                        self.set_pan(pan);
                    }
                    if self.current_move() {
                        self.flush_enter();
                        if let Some(pixel) = pixel {
                            if let Some(MoveInput::Motion(previous)) = self.move_inputs.back_mut() {
                                *previous = pixel;
                            } else {
                                self.move_inputs.push_back(MoveInput::Motion(pixel));
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
            return Task::none();
        }
        if let Message::DialogKey(key) = message {
            if key == 32
                && self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.simulation.vm.world.dialogs.modal.is_none())
            {
                return resident_shortcuts::next(self)
                    .map_or_else(Task::none, |resident| self.update(Message::Resident(resident)));
            }
            if key == 27
                && self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.simulation.vm.world.dialogs.modal.is_none())
            {
                if self
                    .session
                    .as_ref()
                    .and_then(|s| s.host.presentation.ground_input.as_ref())
                    .is_some_and(|i| i.can_cancel())
                {
                    return self.update(Message::GroundCancel);
                }
                if self
                    .session
                    .as_ref()
                    .and_then(|s| s.host.presentation.object_input.as_ref())
                    .is_some_and(|i| i.can_cancel())
                {
                    return self.update(Message::ObjectCancel);
                }
            }
            if key == 27
                && self.current_move()
                && self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.simulation.vm.world.dialogs.modal.is_none())
            {
                self.flush_enter();
                self.move_inputs.push_back(MoveInput::Button {
                    kind: sims_runtime::original_session::move_tool::button::Kind::Cancel,
                    pixel: [0., 0.],
                    picked: 0,
                    shift: false,
                });
                return Task::none();
            }

            let response = self.session.as_ref().and_then(|s| {
                let dialogs = &s.simulation.vm.world.dialogs;
                let record = dialogs
                    .records
                    .iter()
                    .find(|r| Some(r.id) == dialogs.modal)?;
                Some((record.id, record.key_result(key)?))
            });
            return response.map_or_else(Task::none, |(id, result)| {
                self.update(Message::Dialog(id, result))
            });
        }
        if (self.presentation_pending()
            || self
                .session
                .as_ref()
                .is_some_and(Session::player_input_pending))
            && matches!(
                message,
                Message::Queue(..)
                    | Message::Sell(_)
                    | Message::Live
                    | Message::Buy
                    | Message::ExpandPanel
                    | Message::Needs
                    | Message::MotiveDetails(..)
                    | Message::CloseMotiveDetails
                    | Message::Scroll(..)
                    | Message::Cancel(..)
                    | Message::Resident(..)
                    | Message::Rotate
                    | Message::RotateBack
                    | Message::ZoomIn
                    | Message::ZoomOut
                    | Message::Select(..)
                    | Message::ViewportClick(..)
                    | Message::Floor(..)
                    | Message::Walls(_)
                    | Message::Zoom(..)
            )
        {
            return Task::none();
        }
        if self.session.as_ref().is_some_and(Session::running_menu)
            && matches!(message, Message::Toggle | Message::Step | Message::Speed(_))
        {
            return Task::none();
        }
        // A modal is a legitimate child of an elapsed frame. Its answer must
        // not be blocked merely because the parent clock has not returned.
        if self.presentation_children_pending() && matches!(message, Message::Dialog(..)) {
            return Task::none();
        }
        //Reject an ended cursor lifetime before the generic refresh/menu path
        //can publish any work on behalf of a stale button event.
        if let Message::Sell(held) = message {
            if !matches!(self.sell_message(),Some(Message::Sell(h))if h==held) {
                return Task::none();
            }
        }
        if matches!(message, Message::GroundChoice(_))
            && !self
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.ground_input.as_ref())
                .is_some_and(|s| s.ready())
        {
            return Task::none();
        }
        if matches!(
            message,
            Message::Needs | Message::MotiveDetails(_) | Message::CloseMotiveDetails
        ) {
            if self.options_open || self.session.as_ref().is_none_or(|session|
                session.simulation.vm.world.dialogs.modal.is_some()) {
                return Task::none();
            }
            let available = self
                .client
                .as_ref()
                .and_then(|c| c.live_panel())
                .is_some_and(|p| match message {
                    Message::CloseMotiveDetails => p.popup().is_some(),
                    Message::MotiveDetails(index) => {
                        index < 8
                            && p.selected()
                            && self
                                .device
                                .as_ref()
                                .is_none_or(|d| d.panel_state() != Some(1))
                    }
                    _ => {
                        p.person().is_some()
                            && self
                                .device
                                .as_ref()
                                .is_none_or(|d| d.panel_state() != Some(1))
                    }
                });
            if !available {
                return Task::none();
            }
        }
        if matches!(message, Message::GroundCancel)
            && !self
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.ground_input.as_ref())
                .is_some_and(|s| s.can_cancel())
        {
            return Task::none();
        }
        // Ignore old preview-menu events in native Live mode. Only records
        // retained by the live object owner may enter45d20b here.
        if self.device.is_some() && matches!(message, Message::Queue(..)) {
            return Task::none();
        }
        if matches!(message, Message::ObjectChoice(_))
            && !self
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.object_input.as_ref())
                .is_some_and(|s| s.ready())
        {
            return Task::none();
        }
        if matches!(message, Message::ObjectCancel)
            && !self
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.object_input.as_ref())
                .is_some_and(|s| s.can_cancel())
        {
            return Task::none();
        }
        if matches!(message, Message::Restart | Message::ConfiguredRestart(_)) {
            if let Message::ConfiguredRestart(choice) = message {
                choice.stage(&mut self.args);
            }
            self.application_confirmation = None;
            self.options_open = false;
            self.args.load = None;
            self.args.at_tick = 0;
            self.application = neighborhood_usability::Snapshot::loading(&self.args);
            self.viewport_enter_requested = false;
            self.move_inputs.clear();
            let work = self.args.clone();
            self.playing = false;
            self.session = None;
            self.client = None;
            self.device = None;
            self.pan = [0, 0];
            self.scene = None;
            self.status = "Reloading original household…".into();
            self.error = None;
            return Task::perform(
                async move {
                    Ready::load(&work)
                        .map(Arc::new)
                        .map_err(|e| format!("{e:#}"))
                },
                Message::Loaded,
            );
        }
        let frame = matches!(message, Message::Frame(_));
        if matches!(message, Message::Load) {
            self.application_confirmation = None;
            self.options_open = false;
        }
        let outcome = (|| -> Result<()> {
            let activated = match &message {
                Message::Resident(id) => self
                    .session
                    .as_ref()
                    .and_then(|s| live_resident(s, *id))
                    .map(|r| ButtonKey::Resident(r.neighbor)),
                Message::Live => Some(ButtonKey::Control(2010)),
                Message::Buy => Some(ButtonKey::Control(2008)),
                Message::GameOptions => Some(ButtonKey::Control(2009)),
                Message::GameOptionsSection(_, section) => Some(ButtonKey::Control(section.resource())),
                Message::GameOptionsSave(_) => Some(ButtonKey::Control(4950)),
                Message::Needs => Some(ButtonKey::Control(4500)),
                Message::Personality(_) => Some(ButtonKey::Control(4501)),
                Message::Relationships(_) => Some(ButtonKey::Control(4504)),
                Message::Job(_) => Some(ButtonKey::Control(4503)),
                Message::PersonDetails(..) | Message::ClosePersonDetails(..) => Some(ButtonKey::Unpictured),
                Message::MotiveDetails(_) | Message::CloseMotiveDetails => {
                    Some(ButtonKey::Unpictured)
                }
                Message::Rotate => Some(ButtonKey::Control(2011)),
                Message::RotateBack => Some(ButtonKey::Control(2012)),
                Message::ZoomIn => Some(ButtonKey::Control(2013)),
                Message::ZoomOut => Some(ButtonKey::Control(2014)),
                Message::Floor(1) => Some(ButtonKey::Control(2023)),
                Message::Floor(2) => Some(ButtonKey::Control(2024)),
                Message::Walls(1) => Some(ButtonKey::Control(2006)),
                Message::Walls(4) => Some(ButtonKey::Control(2004)),
                Message::Walls(2) => Some(ButtonKey::Control(2007)),
                // Transport/save/dialog controls have no registered original
                // bitmap button in this backend; they do not synthesize one.
                _ => None,
            };
            if let (Some(key), Some(client)) = (activated, self.client.as_mut()) {
                if !client.pending() {
                    client.activate_button(key)?;
                }
            }
            match message {
                Message::Loaded(result) => self.install(result.map_err(anyhow::Error::msg)?)?,
                Message::Frame(now) => {
                    if self.error.is_some() {
                        return Ok(());
                    }
                    if self
                        .session
                        .as_ref()
                        .is_some_and(Session::person_draw_preparation_pending)
                    {
                        // A cold-restored draw caller returns before polling
                        // application input or advancing its elapsed clock.
                        return self.prepare_and_publish();
                    }
                    // Queued host events must not move a monotonic epoch back.
                    let now = self.last.map_or(now, |last| now.max(last));
                    if let (Some(last), Some(device)) = (self.last, self.device.as_mut()) {
                        if device.mode_window().is_some() {
                            device.advance_mode_clock(now.duration_since(last))?;
                        }
                    }
                    let dt = self
                        .last
                        .replace(now)
                        .map_or(0., |last| now.duration_since(last).as_secs_f32())
                        .min(0.25);
                    self.update_clock_time(now)?;
                    if !self.advance_camera_motion_frame()? {
                        return self.prepare_and_publish();
                    }
                    if self.presentation_pending() {
                        if self.native_clock() {
                            self.advance_native_frame_work()?;
                        } else {
                            self.step()?;
                        }
                    } else if self.session.is_some() {
                        if let Some(displacement) = self.viewport_drag.take_scroll() {
                            self.pan = [0, 0];
                            let complete = self.scroll_pixels(displacement)?;
                            let pending = self.presentation_pending();
                            self.viewport_drag.set_camera_owned(!complete || pending);
                            if !complete || pending {
                                return self.prepare_and_publish();
                            }
                        }
                        if !self.viewport_drag.suppresses_pan() && self.pan != [0, 0] && dt > 0. {
                            let pixels = (480. * dt).round().max(1.) as i32;
                            if !self.scroll_pixels(self.pan.map(|n| n * pixels))? {
                                return self.prepare_and_publish();
                            }
                        }
                        if !self.poll_application_ui()? {
                            return Ok(());
                        }
                        let partial = self.native_partial_pass()?;
                        if !self.playing && !partial {
                            return self.prepare_and_publish();
                        }
                        if self.native_clock() {
                            // One original elapsed caller per application frame.
                            // Its saved timer decides how many full/partial lot
                            // passes are due; do not multiply wall time here.
                            self.advance_native_frame_work()?;
                            return self.prepare_and_publish();
                        }
                        // Wall-clock cadence remains the replacement client's
                        // policy; original46d553 decides full/partial execution.
                        let rate = if self.playing { self.speed } else { 1. };
                        self.accumulator = (self.accumulator + dt * rate).min(0.32);
                        while self.accumulator >= 0.04 && (self.playing || partial) {
                            if !self.step()? {
                                break;
                            }
                            self.accumulator -= 0.04;
                        }
                    }
                }
                Message::Pan(direction) => self.set_pan(direction),
                Message::Scroll(displacement) => {
                    self.scroll_pixels(displacement)?;
                }
                Message::Toggle => {
                    let playing = !self.playing && self.error.is_none();
                    if self.native_transport() && self.error.is_none() {
                        self.session.as_mut().unwrap().begin_pause(!playing)?;
                    }
                    self.playing = playing;
                    self.accumulator = 0.;
                }
                Message::Live => {
                    if self
                        .session
                        .as_ref()
                        .and_then(|s| s.host.presentation.transport.as_ref())
                        .is_some_and(|t| t.controls.mode != 2)
                    {
                        let session = self.session.as_mut().unwrap();
                        let playing = *session.simulation.vm.world.globals.get(&17)
                            .context("Live return native pause state absent")? == 0;
                        session.begin_mode_change(2, 0)?;
                        self.playing = playing;
                        self.accumulator = 0.;
                    } else {
                        self.device
                            .as_ref()
                            .context("panel device absent")?
                            .begin_panel_toggle(
                                self.session.as_mut().context("household still loading")?,
                            )?;
                    }
                }
                Message::Buy => {
                    self.session
                        .as_mut()
                        .context("household still loading")?
                        .begin_mode_change(1, 0)?;
                }
                Message::GameOptions => {
                    self.session.as_mut().context("household still loading")?.begin_mode_change(3, 0)?;
                }
                Message::GameOptionsSection(revision, section) => {
                    self.device.as_mut().context("Options device absent")?.select_options_section(
                        self.session.as_ref().context("household still loading")?, revision, section)?;
                }
                Message::ExpandPanel => {
                    self.device
                        .as_ref()
                        .context("panel device absent")?
                        .begin_panel_expansion(
                            self.session.as_mut().context("household still loading")?,
                            0,
                        )?;
                }
                Message::Step => {
                    self.playing = false;
                    if !self.presentation_pending() && !self.poll_application_ui()? {
                        return Ok(());
                    }
                    self.step()?;
                }
                Message::BuyNavigate(revision, event) => {
                    self.device
                        .as_ref()
                        .context("catalog device absent")?
                        .begin_buy_navigation(
                            self.session.as_mut().context("household still loading")?,
                            revision,
                            event,
                        )?;
                }
                Message::BuyProduct(revision, index) => {
                    self.device
                        .as_mut()
                        .context("catalog device absent")?
                        .begin_buy_product(
                            self.session.as_mut().context("household still loading")?,
                            revision,
                            index,
                        )?;
                }
                Message::Speed(speed) => self.speed = speed,
                Message::NativeSpeed(speed) => {
                    self.session
                        .as_mut()
                        .context("household still loading")?
                        .begin_speed(speed)?;
                    self.playing = true;
                    self.accumulator = 0.;
                }
                Message::Floor(floor) => {
                    if self.device.is_some() {
                        self.change_view(Change::Floor(floor))?;
                    } else {
                        self.floor = floor;
                    }
                    self.selected = None;
                }
                Message::Walls(walls) => {
                    if let Some(device) = &self.device {
                        ensure!(
                            !device.normal_view_frame_pending(),
                            "normal-frame owns the published surface"
                        );
                        self.session
                            .as_mut()
                            .context("household still loading")?
                            .begin_walls_change(walls)?;
                        self.status = "Updating view…".into();
                    }
                }
                Message::Zoom(zoom) => {
                    if self.device.is_some() {
                        self.change_view(Change::Zoom(if zoom < 0.375 {
                            1
                        } else if zoom < 0.75 {
                            2
                        } else {
                            3
                        }))?;
                    } else {
                        self.zoom = zoom;
                    }
                }
                Message::Rotate => {
                    if let Some(device) = &self.device {
                        self.change_view(Change::Rotation((device.controls().rotation + 1) & 3))?;
                    } else if let Some(s) = &mut self.session {
                        s.simulation.vm.world.camera_rotation =
                            (s.simulation.vm.world.camera_rotation + 1) % 4;
                    }
                }
                Message::RotateBack => {
                    if let Some(device) = &self.device {
                        self.change_view(Change::Rotation((device.controls().rotation + 3) & 3))?;
                    } else if let Some(s) = &mut self.session {
                        s.simulation.vm.world.camera_rotation =
                            (s.simulation.vm.world.camera_rotation + 3) % 4;
                    }
                }
                Message::ZoomIn => {
                    if self.device.is_some() {
                        self.change_view(Change::ZoomIn)?;
                    } else {
                        self.zoom = (self.zoom * 2.).min(1.5);
                    }
                }
                Message::ZoomOut => {
                    if self.device.is_some() {
                        self.change_view(Change::ZoomOut)?;
                    } else {
                        self.zoom = (self.zoom * 0.5).max(0.15);
                    }
                }
                Message::DisplaySize(_)
                | Message::Shift(_)
                | Message::FocusLost
                | Message::ViewportButton { .. }
                | Message::ViewportRightButton { .. }
                | Message::ViewportEnter
                | Message::ViewportLeave
                | Message::ViewportMotion { .. } => {
                    unreachable!("pointer events are queued before dispatch")
                }
                Message::ViewportClick(pixel) => {
                    let scene = self.scene.as_ref().context("viewport scene absent")?;
                    if let Some(id) = scene.pick(pixel) {
                        self.select_object(Some(id))?;
                    } else {
                        let s = self.session.as_mut().context("household still loading")?;
                        ensure!(
                            self.device.is_some(),
                            "original ground input requires the native view"
                        );
                        let camera = OriginalHouse::native_camera(s)?;
                        let lot = s
                            .simulation
                            .architecture
                            .as_ref()
                            .and_then(|a| a.original.as_ref())
                            .context("ground architecture absent")?;
                        if let Some(point) = camera.pick_original_ground_in(
                            lot,
                            sims_render::original_lot_size(&s.simulation)?,
                            pixel,
                        )? {
                            let mode = s
                                .host
                                .presentation
                                .presenter
                                .as_ref()
                                .unwrap()
                                .camera
                                .as_ref()
                                .unwrap()
                                .settings
                                .application_mode;
                            ensure!(mode == 2, "ground interactions require Live mode");
                            let opened = s.begin_ground_input(point, camera.level)?;
                            self.selected = None;
                            if opened {
                                self.pan = [0, 0];
                                self.accumulator = 0.;
                                self.status = "Preparing movement…".into();
                            }
                        }
                    }
                }
                Message::GroundChoice(index) => {
                    let s = self.session.as_mut().context("household still loading")?;
                    if s.running_menu() {
                        if !s.menu_command_pending() {
                            s.submit_menu_command(
                                sims_runtime::original_session::live_menu::Command::Choose(index),
                            )?;
                        }
                        return Ok(());
                    }
                    let (_, accepted) = s.choose_ground_action(index)?;
                    self.accumulator = 0.;
                    self.status = if accepted {
                        "Interaction queued."
                    } else {
                        "Interaction could not be queued."
                    }
                    .into();
                }
                Message::GroundCancel => {
                    let s = self.session.as_mut().context("household still loading")?;
                    if s.running_menu() {
                        if !s.menu_command_pending() {
                            s.submit_menu_command(
                                sims_runtime::original_session::live_menu::Command::Cancel,
                            )?;
                        }
                        return Ok(());
                    }
                    self.session
                        .as_mut()
                        .context("household still loading")?
                        .cancel_ground_input()?;
                    self.error = None;
                    self.status = "Closing movement menu…".into();
                }
                Message::ObjectChoice(index) => {
                    let s = self.session.as_mut().context("household still loading")?;
                    if s.running_menu() {
                        if !s.menu_command_pending() {
                            s.submit_menu_command(
                                sims_runtime::original_session::live_menu::Command::Choose(index),
                            )?;
                        }
                        return Ok(());
                    }
                    let (_, accepted) = self
                        .session
                        .as_mut()
                        .context("household still loading")?
                        .choose_object_action(index)?;
                    self.selected = None;
                    self.accumulator = 0.;
                    self.status = if accepted {
                        "Interaction queued."
                    } else {
                        "Interaction could not be queued."
                    }
                    .into();
                }
                Message::ObjectCancel => {
                    let s = self.session.as_mut().context("household still loading")?;
                    if s.running_menu() {
                        if !s.menu_command_pending() {
                            s.submit_menu_command(
                                sims_runtime::original_session::live_menu::Command::Cancel,
                            )?;
                        }
                        return Ok(());
                    }
                    self.session
                        .as_mut()
                        .context("household still loading")?
                        .cancel_object_input()?;
                    self.selected = None;
                    self.error = None;
                    self.status = "Interaction menu closed.".into();
                }
                Message::Select(target) => self.select_object(target)?,
                Message::Resident(id) => {
                    let repeated = self.device.is_some()
                        && self
                            .session
                            .as_ref()
                            .context("household still loading")?
                            .simulation
                            .native_selected_person()
                            .map_err(anyhow::Error::msg)?
                            == Some(id);
                    if repeated {
                        let target = resident_portrait::target(
                            &self.session.as_ref().unwrap().simulation.vm.world,
                            id,
                        )?;
                        self.resident = id;
                        if let Some(target) = target {
                            self.change_view(Change::CenterPerson {
                                point: target.point,
                                level: target.level,
                            })?;
                        }
                    } else {
                        self.select_resident(id)?;
                        let s = self.session.as_ref().unwrap();
                        if let Some(level) = s.simulation.bodies.get(&id).map(|b| b.level) {
                            if self.device.is_some() {
                                self.change_view(Change::Floor(level))?;
                            } else {
                                self.floor = level;
                            }
                        }
                    }
                }
                Message::Needs => {
                    self.client
                        .as_mut()
                        .context("live widget client absent")?
                        .select_needs(self.session.as_ref().context("household still loading")?)?;
                }
                Message::Personality(identity) | Message::Relationships(identity) | Message::Job(identity) => {
                    let category = person_panel::request(&message).and_then(|(_, category)| category)
                        .context("person category command absent")?;
                    self.client.as_mut().context("live widget client absent")?.select_person_category(
                        self.session.as_ref().context("household still loading")?, identity, category)?;
                }
                Message::PersonDetails(identity, detail) => {
                    self.client.as_mut().context("live widget client absent")?.toggle_person_details(
                        self.session.as_ref().context("household still loading")?, identity, detail)?;
                }
                Message::ClosePersonDetails(identity, expected) => {
                    self.client.as_mut().context("live widget client absent")?.close_person_details(
                        self.session.as_ref().context("household still loading")?, identity, expected)?;
                }
                Message::MotiveDetails(index) => {
                    self.client
                        .as_mut()
                        .context("live widget client absent")?
                        .toggle_motive_details(
                            self.session.as_ref().context("household still loading")?,
                            index,
                        )?;
                }
                Message::CloseMotiveDetails => {
                    self.client
                        .as_mut()
                        .context("live widget client absent")?
                        .close_motive_details()?;
                }
                Message::Queue(target, index) => {
                    let s = self.session.as_mut().context("household still loading")?;
                    s.simulation
                        .enqueue(&s.program, self.resident, target, index)
                        .map_err(anyhow::Error::msg)?;
                    self.status = "Interaction queued.".into();
                }
                Message::Cancel(id) => {
                    let s = self.session.as_mut().context("household still loading")?;
                    s.simulation
                        .cancel_interaction(&s.program, self.resident, id)
                        .map_err(anyhow::Error::msg)?;
                    self.status = "Cancellation requested.".into();
                }
                Message::Dialog(id, result) => {
                    let vm = &mut self.session
                        .as_mut()
                        .context("household still loading")?
                        .simulation
                        .vm;
                    let answer = window_observation::answer_dialog(
                        vm, id, result, super::support::window_session::enabled(),
                    )
                        .map_err(anyhow::Error::msg)?;
                    if let Some(mut answer) = answer {
                        self.window_dialog_answers += 1;
                        answer["sequence"] = self.window_dialog_answers.into();
                        super::support::window_session::emit("dialog_answer", answer.clone());
                        self.window_dialog_answer = Some(answer);
                    }
                    self.status = "Response recorded.".into();
                }
                Message::Save | Message::GameOptionsSave(_) => {
                    self.save()?;
                    self.status = format!("Saved {}", self.args.save.display());
                }
                Message::Sell(_) => {
                    self.session.as_mut().unwrap().begin_move_tool_sale()?;
                    self.status = "Selling furniture…".into();
                }
                Message::Load => {
                    self.load(&self.args.save.clone())?;
                    self.status = "Saved household restored.".into();
                }
                Message::Restart
                | Message::ConfiguredRestart(_)
                | Message::RequestApplicationChoice(_)
                | Message::CancelApplicationChoice(_)
                | Message::ConfirmApplicationChoice(_)
                | Message::DialogKey(_)
                | Message::Diagnostics
                | Message::Options => unreachable!(),
            }
            self.prepare_and_publish()?;
            Ok(())
        })();
        if let Err(error) = outcome {
            self.fail(format!("{error:#}"));
        }
        if !frame {
            self.menu();
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Message> {
        widget_lifetimes::retain(
            self.widget_epoch,
            neighborhood_usability::recovery(self.view_content(), self.error.is_some()),
        )
    }

    fn view_content(&self) -> Element<'_, Message> {
        let title = self
            .session
            .as_ref()
            .and_then(|s| s.house.household.as_ref())
            .map_or("Original household", |h| h.name.as_str());
        let mut toolbar = row![
            // Native bitmap2026 is a display control, not this transport button.
            published_control(
                if self.playing { "Pause" } else { "Play" },
                Message::Toggle,
                self.device
                    .as_ref()
                    .and_then(|d| d.panel_widgets())
                    .map(|w| w.pause()),
                true,
            ),
            button(if self.native_clock() {
                "Step frame"
            } else if self.native_transport() {
                "Step pass"
            } else {
                "Step"
            })
            .on_press(Message::Step)
        ]
        .spacing(8)
        .align_y(iced::Alignment::End);
        if self
            .device
            .as_ref()
            .is_some_and(|d| d.panel_state().is_some())
        {
            toolbar = toolbar.push(self.control(2010, "Live Mode", Message::Live));
        }
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.host.presentation.tool_selection.is_some())
        {
            toolbar = toolbar.push(self.control(2008, "Buy Mode", Message::Buy));
        }
        if self.native_clock() {
            use sims_vm::interface::speed::{FAST, NORMAL, ULTRA};
            let current = self
                .session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .speed;
            for (speed, label) in [(NORMAL, "Normal"), (FAST, "Fast"), (ULTRA, "Ultra")] {
                use sims_vm::interface::view_control::Widget;
                let kind = match speed {
                    NORMAL => Widget::Normal,
                    FAST => Widget::Fast,
                    _ => Widget::Ultra,
                };
                toolbar = toolbar.push(published_control(
                    label,
                    Message::NativeSpeed(speed),
                    self.device
                        .as_ref()
                        .and_then(|d| d.panel_widgets())
                        .map(|w| w.button(kind)),
                    current == speed,
                ));
            }
        } else {
            for speed in [1., 2., 4., 16.] {
                toolbar = toolbar.push(
                    button(text(format!("{speed:.0}×")))
                        .on_press(Message::Speed(speed))
                        .style(if self.speed == speed {
                            button::primary
                        } else {
                            button::secondary
                        }),
                );
            }
        }
        toolbar = toolbar
            .push(button("Save").on_press(Message::Save))
            .push(button("Load").on_press(Message::Load))
            .push(button("Restart").on_press(Message::Restart))
            .push(self.control(2012, "Rotate left", Message::RotateBack))
            .push(self.control(2011, "Rotate right", Message::Rotate))
            .push(self.control(2013, "Zoom in", Message::ZoomIn))
            .push(self.control(2014, "Zoom out", Message::ZoomOut))
            .push(self.control(2023, "Downstairs", Message::Floor(1)))
            .push(self.control(2024, "Upstairs", Message::Floor(2)));
        let pause_display = self.device.as_ref().and_then(CameraDevice::transport_pause);
        let status = if pause_display.is_some_and(|v| v != 0) {
            "PAUSED"
        } else if self.playing {
            "PLAYING"
        } else if pause_display.is_some() {
            // A diagnostic Step/load hold is separate from the lot pause word.
            "STOPPED"
        } else {
            "PAUSED"
        };
        let mut panel = column![text(status).size(14)].spacing(12);
        if let Some(s) = &self.session {
            if let Some(texts) = self
                .device
                .as_ref()
                .and_then(|d| d.panel_widgets())
                .and_then(|w| w.texts())
            {
                let highlighted = self
                    .device
                    .as_ref()
                    .unwrap()
                    .panel_widgets()
                    .unwrap()
                    .button(sims_vm::interface::view_control::Widget::Status)
                    .selected;
                let mut funds = text(
                    household_funds_label(s, texts.funds())
                        .unwrap_or_else(|_| "Funds unavailable".into()),
                );
                if highlighted {
                    funds = funds.color(Color::from_rgb8(70, 184, 159));
                }
                panel = panel.push(text(household_clock_label(s).unwrap_or_default()));
                let access = self
                    .device
                    .as_ref()
                    .unwrap()
                    .panel_widgets()
                    .unwrap()
                    .access();
                let mut money = row![].spacing(8);
                if let Some(access) = access.filter(|a| a.label_visible()) {
                    money = money.push(text(access.secondary_text()));
                }
                if access.is_none_or(|a| a.funds_visible()) {
                    money = money.push(funds);
                }
                panel = panel.push(money);
            } else if let Some(h) = &s.simulation.household {
                panel = panel.push(text(format!(
                    "Day {} · {:02}:{:02}",
                    h.clock.elapsed_days + 1,
                    h.clock.hour,
                    h.clock.minute
                )));
            }
            if self.people_panel_visible() {
                if let Some(h) = &s.house.household {
                    for r in &h.residents {
                        let identity = resident_object(s, r);
                        let portrait = self.art.as_ref().and_then(|i| i.resident(r.neighbor));
                        let content: Element<'_, Message> = if let Some(image) = portrait {
                            row![widget::image(image).width(45).height(45), text(&r.name)]
                                .spacing(10)
                                .align_y(iced::Alignment::Center)
                                .into()
                        } else {
                            text(&r.name).into()
                        };
                        let highlighted = self
                            .client
                            .as_ref()
                            .and_then(|c| c.buttons())
                            .is_some_and(|b| b.highlighted(r.neighbor));
                        if highlighted {
                            panel = panel.push(
                                widget::image(self.art.as_ref().unwrap().tutorial())
                                    .width(50)
                                    .height(50),
                            );
                        }
                        panel = panel.push(
                            button(content)
                                .on_press_maybe(
                                    identity.filter(|identity| s.simulation.has_interaction_actor(*identity))
                                        .map(Message::Resident),
                                )
                                .style(if identity == Some(self.resident) {
                                    button::primary
                                } else {
                                    button::secondary
                                }),
                        );
                    }
                }
                if let Some(needs) = self.client.as_ref().and_then(|c| c.live_panel()) {
                    if self
                        .device
                        .as_ref()
                        .is_none_or(|d| d.panel_state() != Some(1))
                    {
                        if self.tutorial_highlight == Some(4500) {
                            if let Some(icons) = &self.art {
                                panel = panel
                                    .push(widget::image(icons.tutorial()).width(50).height(50));
                            }
                        }
                        panel = panel.push(person_panel::sidebar(needs,
                            !self.presentation_pending() && s.simulation.vm.world.dialogs.modal.is_none()
                                && !self.options_open, self.tutorial_highlight));
                    }
                } else if let Some(m) = s
                    .simulation
                    .vm
                    .world
                    .objects
                    .get(&self.resident)
                    .filter(|_| {
                        self.device
                            .as_ref()
                            .is_none_or(|d| d.panel_state() != Some(1))
                    })
                    .filter(|_| live_resident(s, self.resident).is_some())
                    .and_then(|o| o.motives)
                {
                    for pair in [
                        [("Hunger", 7), ("Energy", 5)],
                        [("Comfort", 6), ("Hygiene", 8)],
                        [("Bladder", 9), ("Fun", 15)],
                        [("Social", 14), ("Room", 13)],
                    ] {
                        panel = panel.push(
                            row(pair.into_iter().map(|(name, i)| {
                                column![
                                    text(name).size(12),
                                    progress_bar(
                                        0.0..=1.0,
                                        ((f32::from_bits(m[i]) + 100.) / 200.).clamp(0., 1.)
                                    )
                                    .girth(7)
                                ]
                                .width(Fill)
                                .spacing(4)
                                .into()
                            }))
                            .spacing(14),
                        );
                    }
                }
                if let Some(input) = &s.host.presentation.ground_input {
                    panel = panel.push(text("Move here").size(18));
                    if input.ready() {
                        for (index, action) in input.actions().iter().enumerate() {
                            let label = s
                                .simulation
                                .vm
                                .action_menu(&s.program, action.target)
                                .ok()
                                .and_then(|table| table.iter().find(|e| e.id == action.menu_id))
                                .map(|e| e.label.as_str())
                                .unwrap_or("");
                            panel = panel.push(
                                button(text(label))
                                    .width(Fill)
                                    .on_press(Message::GroundChoice(index)),
                            );
                        }
                    } else {
                        panel = panel.push(text("Preparing interaction…").size(13));
                    }
                    if input.can_cancel() {
                        panel = panel.push(button("Cancel").on_press(Message::GroundCancel));
                    }
                } else if let Some(input) = &s.host.presentation.object_input {
                    panel = panel.push(text(self.label(input.target())).size(18));
                    if input.ready() {
                        for (index, action) in input.actions().iter().enumerate() {
                            let label = s
                                .simulation
                                .vm
                                .action_menu(&s.program, action.target)
                                .ok()
                                .and_then(|table| table.iter().find(|e| e.id == action.menu_id))
                                .map(|e| e.label.as_str())
                                .unwrap_or("");
                            panel = panel.push(
                                button(text(label))
                                    .width(Fill)
                                    .on_press(Message::ObjectChoice(index)),
                            );
                        }
                    } else {
                        panel = panel.push(text("Preparing interaction…").size(13));
                    }
                    if input.can_cancel() {
                        panel = panel.push(button("Cancel").on_press(Message::ObjectCancel));
                    }
                } else if let Some(id) = self.selected {
                    panel = panel.push(text(self.label(id)).size(18));
                    for c in &self.choices {
                        panel = panel.push(
                            button(text(&c.label)).width(Fill).on_press_maybe(
                                (c.available && live_resident(s, self.resident).is_some())
                                    .then_some(Message::Queue(id, c.index)),
                            ),
                        );
                    }
                    if let Some(error) = &self.menu_error {
                        panel = panel.push(text(error).size(12));
                    }
                } else {
                    panel = panel.push(text("Select an object in the room.").size(13));
                }
                panel = panel.push(text("INTERACTION QUEUE").size(13));
                if let Some(actor) = s
                    .simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .filter(|_| live_resident(s, self.resident).is_some())
                    .and_then(|a| a.actors.get(&self.resident))
                {
                    for action in actor.active.iter().chain(actor.queue.pending.iter()) {
                        let label = action
                            .label
                            .clone()
                            .filter(|s| !s.is_empty())
                            .or_else(|| {
                                super::support::entries(&s.program, &s.simulation, action.target)
                                    .iter()
                                    .find(|e| e.id == action.menu_id)
                                    .map(|e| e.label.clone())
                            })
                            .unwrap_or_else(|| "Interaction".into());
                        panel = panel.push(
                            row![
                                text(format!("{label} · {}", self.label(action.target)))
                                    .size(13)
                                    .width(Fill),
                                button("×").on_press(Message::Cancel(action.id.into()))
                            ]
                            .spacing(8),
                        );
                    }
                }
            } else if let Some(active) = self
                .device
                .as_ref()
                .and_then(|d| d.panel_widgets())
                .and_then(|w| w.presentation())
                .and_then(|p| p.active())
            {
                use sims_render::camera_device::panels::Panel;
                panel = panel.push(
                    text(match active {
                        Panel::Buy => "Buy mode",
                        Panel::Build => "Build mode",
                        Panel::Options => "Options",
                        Panel::Camera => "Camera",
                        Panel::Auxiliary => "Details",
                        Panel::People => unreachable!(),
                    })
                    .size(18),
                );
                if active == Panel::Options && let Some(options) = self.device.as_ref().and_then(|device| device.panel_widgets())
                    .and_then(|widgets| widgets.options()).and_then(|options| options.view()) {
                    panel = panel.push(game_options::view(options, self.art.as_deref(), !self.presentation_pending()
                        && s.simulation.vm.world.dialogs.modal.is_none()));
                }
                if active == Panel::Buy
                    && let Some(catalog) = self
                        .device
                        .as_ref()
                        .and_then(|d| d.panel_widgets())
                        .and_then(|w| w.catalog())
                {
                    if let Some(selected) =
                        self.device.as_ref().and_then(|d| d.furniture_selection())
                    {
                        panel = panel
                            .push(text(format!("Moving {}", selected.name)))
                            .push(text(format!("Value: ${}", selected.price)))
                            .push(button("Sell").on_press_maybe(self.sell_message()))
                            .push(button("Cancel move").on_press_maybe(
                                (!self.presentation_pending()).then_some(Message::DialogKey(27)),
                            ));
                    }
                    panel = panel.push(buy_catalog_view(
                        catalog,
                        !self.presentation_pending()
                            && s.simulation.vm.world.dialogs.modal.is_none(),
                    ));
                }
            }
            if self
                .device
                .as_ref()
                .and_then(|d| d.panel_widgets())
                .and_then(|w| w.presentation())
                .is_some_and(|p| !p.root_visible())
            {
                panel = panel.push(
                    button("Show controls").on_press_maybe(
                        (!self.presentation_pending()
                            && s.simulation.vm.world.dialogs.modal.is_none())
                        .then_some(Message::ExpandPanel),
                    ),
                );
            }
            panel = panel.push(
                text(format!(
                    "Tick {} (+{})",
                    s.simulation.vm.tick,
                    s.simulation.vm.tick - u64::from(s.house.simulator.ticks)
                ))
                .size(12),
            );
        }
        panel = panel
            .push(text("Zoom").size(12))
            .push(if let Some(device) = &self.device {
                slider(1.0..=3.0, device.controls().zoom as f32, |level| {
                    Message::Zoom((1u32 << level as u32) as f32 / 8.)
                })
                .step(1.0_f32)
            } else {
                slider(0.15..=1.5, self.zoom, Message::Zoom)
            });
        let viewport: Element<_> = if let Some(scene) = &self.scene {
            container(
                widget::mouse_area(
                    widget::shader::Shader::new(Viewport(scene.clone()))
                        .width(Fill)
                        .height(Fill),
                )
                .on_enter(Message::ViewportEnter)
                .on_exit(Message::ViewportLeave)
                .interaction(
                    if self.device.as_ref().and_then(|d| d.tool_cursor()) == Some(0) {
                        iced::mouse::Interaction::Idle
                    } else {
                        iced::mouse::Interaction::None
                    },
                ),
            )
            .padding(iced::Padding {
                bottom: if self.diagnostics {
                    0.
                } else {
                    self.hud_height() as f32
                },
                ..Default::default()
            })
            .into()
        } else {
            container(text("Loading original assets…"))
                .center(Fill)
                .into()
        };
        // This popup is independent of the scrollable/collapsed bar strip.
        // Its original script-visible ownership must correspond to visible UI.
        let viewport: Element<_> = if let Some(detail) = self
            .client
            .as_ref()
            .and_then(|c| c.live_panel())
            .and_then(person_panel::popup)
        {
            widget::stack![
                viewport,
                container(widget::opaque(detail))
                    .width(Fill)
                    .height(Fill)
                    .align_x(iced::alignment::Horizontal::Right)
                    .align_y(iced::alignment::Vertical::Top)
                    .padding(12)
            ]
            .into()
        } else {
            viewport
        };
        let viewport: Element<_> = widget::stack![
            viewport,
            container(self.queue_hud())
                .width(Fill)
                .padding(if self.diagnostics { 10 } else { 33 }),
        ]
        .into();
        let viewport: Element<_> = if let Some(menu) = self.interaction_popup() {
            widget::stack![viewport, container(widget::opaque(menu)).center(Fill)].into()
        } else {
            viewport
        };
        let policy = self
            .session
            .as_ref()
            .map(|s| s.options.routing)
            .unwrap_or_default();
        let page = if self.diagnostics {
            column![
            text(format!("The {title} household")).size(27),
            text(match policy {
                sims_sim::lot_route::path::Policy::Strict =>
                    "Original house · strict routing validation",
                sims_sim::lot_route::path::Policy::InferredRectangles =>
                    "Original house · experimental routing (native rectangle search unverified)",
            })
            .size(13),
            row![toolbar,button("Play view").on_press(Message::Diagnostics)].spacing(8),
            row![viewport, scrollable(panel).width(300)]
                .spacing(16)
                .height(Fill),
            text(&self.status).size(12)
        ]
            .padding(20)
            .spacing(14)
        } else {
            let mut play = widget::stack![
                viewport,
                container(self.household_hud())
                    .width(Fill)
                    .height(Fill)
                    .align_y(iced::alignment::Vertical::Bottom),
            ];
            if self.device.as_ref().is_some_and(|d| {
                d.mode_window()
                    .map_or(d.transport_pause().is_some_and(|v| v != 0), |w| {
                        w.indicator_visible()
                    })
            }) {
                play = play.push(
                    container(
                        text("Paused")
                            .font(iced::Font {
                                family: iced::font::Family::Name("Comic Sans MS"),
                                ..iced::Font::DEFAULT
                            })
                            .size(64)
                            .color(Color::from_rgb8(255, 0, 0)),
                    )
                    .width(Fill)
                    .align_x(iced::alignment::Horizontal::Center)
                    .padding(iced::Padding {
                        top: 8.,
                        ..Default::default()
                    }),
                );
            }
            play = play.push(self.tutorial_hud_overlay());
            play = play.push(neighborhood_usability::notice(&self.application));
            let play: Element<_> = play.into();
            let play = if self.options_open {
                widget::stack![play, self.application_options()].into()
            } else {
                play
            };
            column![play].height(Fill)
        };
        let page: Element<_> = if let Some(error) = &self.error {
            //Feedback overlays the existing layout: an error must not steal
            //height from the shader and rescale its frozen camera surface.
            widget::stack![
                page,
                container(text(error).size(12).color(Color::from_rgb8(245, 131, 115)))
                    .width(Fill)
                    .height(Fill)
                    .align_y(iced::alignment::Vertical::Bottom)
            ]
            .into()
        } else {
            page.into()
        };
        if let (Some(s), Some(icons)) = (&self.session, &self.art) {
            if let Some(record) = s.simulation.vm.world.dialogs.modal.and_then(|id| {
                s.simulation
                    .vm
                    .world
                    .dialogs
                    .records
                    .iter()
                    .find(|r| r.id == id)
            }) {
                return widget::stack![page, dialog_view::overlay(record, icons, Message::Dialog)]
                    .into();
            }
            if let Some(record) = s
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|r| r.status == 1 && !r.modal())
            {
                return widget::stack![page, dialog_view::overlay(record, icons, Message::Dialog)]
                    .into();
            }
        }
        page.into()
    }

    fn control<'a>(&'a self, id: i32, label: &'a str, message: Message) -> Element<'a, Message> {
        let marker: Element<'a, Message> = if self.tutorial_highlight == Some(id) {
            self.art.as_ref().map_or_else(
                || widget::Space::new().width(50).height(50).into(),
                |icons| widget::image(icons.tutorial()).width(50).height(50).into(),
            )
        } else {
            widget::Space::new().width(50).height(50).into()
        };
        // Original5de13f centers the50px highlight three pixels above the
        // target. Iced owns control layout; this shows the initial atlas frame.
        use sims_vm::interface::view_control::Widget;
        let kind = match id {
            2010 => Some(Widget::Live),
            2008 => Some(Widget::Buy),
            2013 => Some(Widget::ZoomIn),
            2014 => Some(Widget::ZoomOut),
            2023 => Some(Widget::FloorOne),
            2024 => Some(Widget::FloorTwo),
            _ => None,
        };
        let published = kind.and_then(|kind| {
            self.device
                .as_ref()
                .and_then(|d| d.panel_widgets())
                .map(|w| w.button(kind))
        });
        column![marker, published_control(label, message, published, true)]
            .spacing(3)
            .align_x(iced::Alignment::Center)
            .into()
    }
}

/// Replacement-client object heading. The linked catalog map is already keyed
/// by OBJD identity, after following that definition's CatalogStringsID. An
/// authentic catalog can contain unrelated copied text (notably Roaches.iff).
/// This is not a claim that the original pie menu displays a catalog heading.
fn object_catalog_label<'a>(
    program: &'a sims_vm::Program,
    world: &sims_vm::World,
    id: i16,
) -> Option<&'a str> {
    let definition = world.objects.get(&id)?.definition.as_ref()?;
    program
        .modules
        .get(&definition.source)?
        .catalog_names
        .get(&definition.object)
        .map(String::as_str)
}

/// Both the interactive toolbar and headless checks read the actual published
/// widget byte/style. Historical devices keep their existing presentation.
fn published_control<'a>(
    label: &'a str,
    message: Message,
    state: Option<sims_render::camera_device::panels::ButtonView>,
    fallback_selected: bool,
) -> Element<'a, Message> {
    button(label)
        .on_press_maybe(state.is_none_or(|s| s.enabled).then_some(message))
        .style(if state.map_or(fallback_selected, |s| s.selected) {
            button::primary
        } else {
            button::secondary
        })
        .into()
}

#[derive(Clone)]
struct Viewport(Arc<Scene>);
impl widget::shader::Program<Message> for Viewport {
    type State = viewport_drag::Pointer;
    type Primitive = Primitive;
    fn update(
        &self,
        pressed: &mut viewport_drag::Pointer,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<widget::shader::Action<Message>> {
        if matches!(event, iced::Event::Window(window::Event::Unfocused)) {
            pressed.reset_right();
        }
        if let Some(message) =
            pressed.right_button(event, [self.0.width, self.0.height], bounds, cursor)
        {
            return Some(widget::shader::Action::publish(message).and_capture());
        }
        if let Some(message) = pressed.motion(event, [self.0.width, self.0.height], bounds, cursor)
        {
            return Some(widget::shader::Action::publish(message));
        }
        let down = match event {
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
                true
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                false
            }
            _ => return None,
        };
        //A drag begun in this viewport must receive its release even when
        //the pointer leaves it. Preserve event coordinates outside its bounds.
        let pixel = if down {
            let pixel = scene_cursor(&self.0, bounds, cursor)?;
            pressed.left = Some(pixel);
            pixel
        } else {
            let prior = pressed.left.take();
            scene_cursor(&self.0, bounds, cursor).or_else(|| {
                prior.map(|last| {
                    let scale = (bounds.width / self.0.width as f32)
                        .min(bounds.height / self.0.height as f32);
                    cursor.position().filter(|_| scale > 0.).map_or(last, |p| {
                        [
                            (p.x - bounds.x - (bounds.width - self.0.width as f32 * scale) / 2.)
                                / scale,
                            (p.y - bounds.y - (bounds.height - self.0.height as f32 * scale) / 2.)
                                / scale,
                        ]
                    })
                })
            })?
        };
        Some(
            widget::shader::Action::publish(Message::ViewportButton {
                down,
                pixel,
                picked: self.0.pick(pixel),
            })
            .and_capture(),
        )
    }
    fn draw(
        &self,
        _: &viewport_drag::Pointer,
        _: iced::mouse::Cursor,
        _: iced::Rectangle,
    ) -> Primitive {
        Primitive {
            scene: self.0.clone(),
        }
    }
}

#[path = "original_viewer/exploration.rs"]
mod exploration;

#[cfg(test)]
#[path = "headless_input.rs"]
mod headless_input;

#[cfg(test)]
#[path = "original_viewer_gui_tests.rs"]
mod gui_tests;

#[cfg(test)]
#[path = "original_viewer_gui_clock_tests.rs"]
mod gui_clock_tests;

// Shared with the headless widget-event driver so subscription coverage uses
// the production mapping, including widget-capture precedence.
fn subscription_event(
    event: iced::Event,
    status: iced::event::Status,
    _: window::Id,
) -> Option<Message> {
    if let iced::Event::Window(window::Event::Resized(size)) = event {
        return Some(Message::DisplaySize(size));
    }
    if matches!(event, iced::Event::Window(window::Event::Unfocused)) {
        return Some(Message::FocusLost);
    }
    if let iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) = event {
        return Some(Message::Shift(m.shift()));
    }
    if status == iced::event::Status::Captured {
        return None;
    }
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    match key.as_ref() {
        iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft) => {
            Some(Message::Scroll([32, 0]))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowRight) => {
            Some(Message::Scroll([-32, 0]))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
            Some(Message::Scroll([0, 32]))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
            Some(Message::Scroll([0, -32]))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) => {
            Some(Message::DialogKey(13))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
            Some(Message::DialogKey(27))
        }
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Space) => {
            Some(Message::DialogKey(32))
        }
        _ => None,
    }
}

pub fn run(mut args: Args) -> Result<()> {
    if let Some(output) = &args.window_session_output {
        super::support::window_session::start(output, &args.root)?;
    }
    if let Some(library) = &args.window_session_present_library {
        super::support::window_session::start_presentation(library)?;
    }
    if args.exploration_plan.is_some() {
        return exploration::run(args);
    }
    if args.save == Path::new("saves/room-viewer.json") {
        args.save = "saves/original-household.json".into();
    }
    validate_output(&args.root, &args.save)?;
    if let Some(path) = &args.screenshot {
        validate_output(&args.root, path)?;
    }
    if args.headless_screenshot {
        let ready = Arc::new(Ready::load(&args)?);
        let output = args.screenshot.clone().unwrap();
        let (mut viewer, _) = Viewer::new(args);
        viewer.install(ready)?;
        if viewer.native_transport() {
            for _ in 0..10000 {
                if !viewer.session.as_ref().unwrap().transport_pending() {
                    break;
                }
                viewer.step()?;
            }
            ensure!(
                !viewer.session.as_ref().unwrap().transport_pending(),
                "headless pause update exceeded budget"
            );
            viewer.refresh()?;
        }
        render_headless_element(&viewer.args.root, &output, viewer.view())?;
        println!("Saved original-house Iced frame: {}", output.display());
        return Ok(());
    }
    ensure!(
        args.screenshot.is_none(),
        "original-house screenshots currently use --headless-screenshot"
    );
    iced::application(
        move || worker_host::Host::new(args.clone()),
        worker_host::Host::update,
        worker_host::Host::view,
    )
    .title("Sims · Original household")
    .window_size(Size::new(1320., 900.))
    .exit_on_close_request(false)
    .theme(|_: &worker_host::Host| room_theme())
    .subscription(worker_host::Host::subscription)
    .run()?;
    Ok(())
}

#[cfg(test)]
#[path = "original_viewer_sha2_backend_tests.rs"]
mod sha2_backend_tests;

#[cfg(test)]
#[path = "original_viewer_catalog_tests.rs"]
mod catalog_tests;
#[cfg(test)]
#[path = "original_viewer_ground_tests.rs"]
mod ground_tests;
#[cfg(test)]
#[path = "original_viewer_live_widget_tests.rs"]
mod live_widget_tests;
#[cfg(test)]
#[path = "original_viewer_object_preview_tests.rs"]
mod object_preview_tests;
#[cfg(test)]
#[path = "original_viewer_object_tests.rs"]
mod object_tests;
#[cfg(test)]
#[path = "original_viewer_panel_widget_tests.rs"]
mod panel_widget_tests;
#[cfg(test)]
#[path = "original_viewer_sale_tests.rs"]
mod sale_tests;

#[cfg(test)]
#[path = "original_viewer_clock_tests.rs"]
mod clock_tests;
#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "original_viewer_continuation_tests.rs"]
#[rustfmt::skip]
mod continuation_tests;
#[cfg(test)]
#[path = "original_viewer_live_menu_tests.rs"]
mod live_menu_tests;
#[cfg(test)]
#[path = "original_viewer_neighborhood_tests.rs"]
mod neighborhood_tests;
#[cfg(test)]
#[path = "original_viewer_person_tests.rs"]
mod person_tests;
#[cfg(test)]
#[path = "original_viewer_property_tests.rs"]
mod property_tests;
#[cfg(test)]
#[path = "original_viewer_selection_tests.rs"]
mod selection_tests;
#[cfg(test)]
#[path = "original_viewer_smoothness_tests.rs"]
mod smoothness_tests;
#[cfg(test)]
#[path = "original_viewer_transport_tests.rs"]
mod transport_tests;

#[cfg(test)]
#[path = "original_viewer/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../sims-content/tests/support/portrait.rs"]
#[allow(dead_code)]
mod portrait_support;

#[cfg(test)]
#[path = "original_viewer/dialog_tests.rs"]
mod dialog_tests;

#[cfg(test)]
#[path = "original_viewer/visual_tests.rs"]
mod visual_tests;

#[cfg(test)]
#[path = "original_viewer/appearance_tests.rs"]
mod appearance_tests;
#[cfg(test)]
#[path = "original_viewer/career_appearance_tests.rs"]
mod career_appearance_tests;
#[cfg(test)]
#[path = "original_viewer/dynamic_menu_tests.rs"]
mod dynamic_menu_tests;
#[cfg(test)]
#[path = "original_viewer/pool_tests.rs"]
mod pool_tests;
