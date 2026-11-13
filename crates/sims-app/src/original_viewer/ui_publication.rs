//! Complete immutable frontend observation at one ordinary application cut.
//! Capturing reads owners only. It never polls, prepares assets, clones World,
//! serializes a Session, or supplies an engine service result.
use super::*;
use super::super::support::window_session;
use sims_render::camera_device::panels::{ButtonView, Panel};
use sims_vm::interface::view_control::Widget;
use std::collections::{BTreeMap, BTreeSet};

#[path = "ui_publication/view.rs"]
pub(super) mod view;

#[cfg(test)]
#[path = "ui_publication/queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "ui_publication/journey_ground_probe.rs"]
mod journey_ground_probe;
#[cfg(test)]
pub(crate) use journey_ground_probe::{
    JourneyGround, JourneyGroundInput, JourneyGroundPixel, JourneyWalking, JourneyWalkingTarget,
};
#[cfg(test)]
#[path = "ui_publication/journey_object_probe.rs"]
mod journey_object_probe;
#[cfg(test)]
pub(crate) use journey_object_probe::{
    JourneyObject, JourneyObjectIdentity, JourneyObjectInput, JourneyObjectProgress,
};

#[derive(Clone)]
pub(crate) struct Resident {
    pub object: i16,
    pub neighbor: i16,
    pub name: String,
    pub enabled: bool,
    pub highlighted: bool,
}
#[derive(Clone)]
pub(crate) struct Money {
    pub clock: String,
    pub funds: String,
    pub highlighted: bool,
    pub secondary: Option<String>,
    pub funds_visible: bool,
}
#[derive(Clone)]
pub(crate) struct Choice {
    pub label: String,
    pub intent: Option<Intent>,
}
/// Only player command identities may cross the publication boundary. In
/// particular Message::Loaded contains a mutable Ready owner and cannot be
/// represented here, even by an accidental future producer.
#[derive(Clone)]
pub(crate) enum Intent {
    GroundChoice(usize),
    ObjectChoice(usize),
    GroundCancel,
    ObjectCancel,
    Queue(i16, usize),
    ClearSelection,
    Cancel(u64),
    Sell(u32),
}
impl Intent {
    pub fn message(&self) -> Message {
        match *self {
            Self::GroundChoice(i) => Message::GroundChoice(i),
            Self::ObjectChoice(i) => Message::ObjectChoice(i),
            Self::GroundCancel => Message::GroundCancel,
            Self::ObjectCancel => Message::ObjectCancel,
            Self::Queue(id, i) => Message::Queue(id, i),
            Self::ClearSelection => Message::Select(None),
            Self::Cancel(id) => Message::Cancel(id),
            Self::Sell(id) => Message::Sell(id),
        }
    }
}
#[derive(Clone)]
pub(crate) struct Menu {
    pub title: String,
    pub diagnostic_title: String,
    pub preview: bool,
    pub ready: bool,
    pub diagnostic: Vec<Choice>,
    pub popup: Vec<Choice>,
    pub cancel: Option<Intent>,
    pub error: Option<String>,
    pub popup_visible: bool,
}
#[derive(Clone)]
pub(crate) struct QueueEntry {
    pub action: QueueAction,
    pub label: String,
    pub diagnostic_label: String,
    pub target: String,
    pub image: Option<widget::image::Handle>,
}
/// Immutable engine facts, including the resident lifetime and exact action
/// serial/target. CANCELLED records remain visible until the engine removes
/// them; the flag means a cancellation request, not completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QueueAction {
    pub resident: i16,
    pub resident_lifetime: Option<u32>,
    pub serial: u32,
    pub target: i16,
    pub active: bool,
    pub cancel_requested: bool,
}
impl QueueAction {
    fn capture(
        resident: i16,
        resident_lifetime: Option<u32>,
        action: &sims_vm::action::Action,
        active: bool,
    ) -> Self {
        Self {
            resident,
            resident_lifetime,
            serial: action.id,
            target: action.target,
            active,
            cancel_requested: action.flags & sims_vm::action::CANCELLED != 0,
        }
    }
    pub(crate) fn cancel(&self) -> Option<Intent> {
        (!self.cancel_requested).then_some(Intent::Cancel(self.serial.into()))
    }
    fn status(&self) -> &'static str {
        if self.cancel_requested {
            "Cancelling…"
        } else if self.active {
            "Active"
        } else {
            "Queued"
        }
    }
}
#[derive(Clone)]
pub(crate) struct Furniture {
    pub name: String,
    pub price: i32,
    pub sell: Option<Intent>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) enum JourneyTarget {
    Person(i16),
    Object { target: i16, table_sit: bool },
}

#[cfg(test)]
impl JourneyTarget {
    fn capture(
        world: &sims_vm::World,
        program: &sims_vm::Program,
        clicked: i16,
    ) -> std::result::Result<Self, String> {
        let target = sims_vm::multitile::interaction_target(world, program, clicked)?;
        let object = world
            .objects
            .get(&target)
            .ok_or("resolved journey target missing")?;
        if object.person.is_some() {
            return Ok(Self::Person(target));
        }
        let definition = object
            .definition
            .as_ref()
            .ok_or("resolved journey target definition missing")?;
        let module = program
            .modules
            .get(&definition.source)
            .ok_or("resolved journey target module unavailable")?;
        let table = module
            .object_interactions
            .get(&definition.object)
            .ok_or("resolved journey target interaction table unavailable")?;
        Ok(Self::Object {
            target,
            table_sit: table.iter().any(|entry| entry.label == "Sit"),
        })
    }
}

#[cfg(test)]
fn capture_journey_targets(
    app: &Viewer,
) -> BTreeMap<i16, std::result::Result<JourneyTarget, String>> {
    let mut targets = BTreeMap::new();
    if let (Some(scene), Some(session)) = (&app.scene, &app.session) {
        for object in scene.draws.iter().filter_map(|draw| draw.object) {
            if object > 0 {
                targets.entry(object).or_insert_with(|| {
                    JourneyTarget::capture(&session.simulation.vm.world, &session.program, object)
                });
            }
        }
    }
    targets
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct JourneyCameraWatch {
    pub tick: u64,
    pub lesson: Option<sims_vm::dialog::Record>,
    pub camera: Option<sims_sim::pose::portrait::view::Input>,
    pub owner: Option<sims_vm::ObjectId>,
    pub owner_source: Option<String>,
    pub query: Option<[i16; 2]>,
    pub baseline: Option<i32>,
    pub attempts: Option<u32>,
    pub world_rotation: Option<u32>,
    pub poll_pending: Option<bool>,
    pub callback_pending: bool,
}

/// Retain only the currently visible page, not the complete indexed catalog.
#[derive(Clone)]
pub(crate) struct Catalog {
    revision: u64,
    view: Option<sims_render::camera_device::panels::catalog::View>,
    products: BTreeMap<usize, sims_content::object_catalog::products::Product>,
    selected: BTreeSet<usize>,
    insufficient: bool,
    pub images: BTreeMap<u32, widget::image::Handle>,
}
/// Exact identity of the currently rendered native catalog controls. Incidental
/// presentation (thumbnails, funds, insufficient-funds styling) is excluded.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CatalogIdentity {
    revision: u64,
    view: Option<sims_render::camera_device::panels::catalog::View>,
    products: Vec<(usize, u32, sims_vm::Definition)>,
}
impl Catalog {
    fn capture(
        catalog: &sims_render::camera_device::panels::catalog::Catalog,
        app: &Viewer,
    ) -> Self {
        let products: BTreeMap<_, _> = catalog
            .view()
            .into_iter()
            .flat_map(|v| v.products.iter())
            .filter_map(|&index| catalog.product(index).map(|p| (index, p.clone())))
            .collect();
        Self {
            revision: catalog.revision(),
            view: catalog.view().cloned(),
            selected: products
                .keys()
                .copied()
                .filter(|&i| catalog.selected(i))
                .collect(),
            insufficient: catalog.insufficient_funds(),
            images: products
                .values()
                .filter_map(|p| {
                    app.thumbnails
                        .as_ref()?
                        .image(p.definition)
                        .map(|image| (p.definition, image))
                })
                .collect(),
            products,
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn identity(&self) -> CatalogIdentity {
        CatalogIdentity {
            revision: self.revision,
            view: self.view.clone(),
            products: self
                .products
                .iter()
                .map(|(&i, p)| (i, p.definition, p.source.clone()))
                .collect(),
        }
    }
    pub fn view(&self) -> Option<&sims_render::camera_device::panels::catalog::View> {
        self.view.as_ref()
    }
    pub fn product(
        &self,
        index: usize,
    ) -> Option<&sims_content::object_catalog::products::Product> {
        self.products.get(&index)
    }
    pub fn selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }
    pub fn insufficient_funds(&self) -> bool {
        self.insufficient
    }
}

/// No mutable producer, Session, Client, Device, Index, world or program is
/// reachable from this object. Image and Scene Arcs retain coherent versions.
#[derive(Clone)]
pub(crate) struct Published {
    pub application: neighborhood_usability::Snapshot,
    pub scene: Option<Arc<Scene>>,
    pub art: Option<Arc<dialog_view::HudArt>>,
    pub cluster: Option<Arc<hud::cluster::Published>>,
    pub widget_epoch: u64,
    pub title: String,
    pub session_present: bool,
    pub cursor_idle: bool,
    pub native_clock: bool,
    pub native_transport: bool,
    pub native_speed: Option<i32>,
    pub playing: bool,
    pub speed: f32,
    pub zoom: f32,
    pub device_zoom: Option<u32>,
    pub pending: bool,
    pub modal: bool,
    pub pause: Option<u8>,
    pub paused_indicator: bool,
    pub people_visible: bool,
    pub panel_state: Option<i32>,
    pub active_panel: Option<Panel>,
    pub root_visible: bool,
    pub controls: Vec<(Widget, ButtonView)>,
    pub pause_button: Option<ButtonView>,
    pub can_buy: bool,
    pub hud_height: u32,
    pub tutorial_highlight: Option<i32>,
    pub status: String,
    pub save_load_feedback: Option<String>,
    pub player_shortcuts_blocked: bool,
    pub camera_shortcuts_blocked: bool,
    pub load_picker_blocked: bool,
    pub load_seed: std::path::PathBuf,
    pub error: Option<String>,
    pub policy: sims_sim::lot_route::path::Policy,
    pub resident: i16,
    /// Native selection and input eligibility at this same observation cut.
    /// `resident` remains the separate queue/legacy bookkeeping identity.
    pub resident_selection: resident_selection::Snapshot,
    pub residents: Vec<Resident>,
    pub money: Option<Money>,
    pub clock_fallback: Option<String>,
    pub funds: String,
    pub needs: Option<sims_render::dialog::client::live_panel::Panel>,
    pub legacy_motives: Option<Vec<u32>>,
    pub mood: Option<f32>,
    pub menu: Option<Menu>,
    pub queue: Vec<QueueEntry>,
    pub catalog: Option<Catalog>,
    pub furniture: Option<Furniture>,
    pub tick_label: Option<String>,
    pub dialog: Option<sims_vm::dialog::Record>,
    /// Actual retained popup owner, including canonical actor/target/clicked
    /// lifetimes. It is command provenance, not a mutable engine reference.
    pub native_popup: Option<sims_runtime::original_session::live_menu::State>,
    pub native_popup_epoch: u64,
    pub window_observation: Option<Arc<serde_json::Value>>,
    #[cfg(test)]
    pub audit: Option<serde_json::Value>,
    #[cfg(test)]
    pub native_selection: Option<std::result::Result<Option<sims_vm::ObjectId>, String>>,
    #[cfg(test)]
    pub journey_targets: BTreeMap<i16, std::result::Result<JourneyTarget, String>>,
    #[cfg(test)]
    pub journey_camera: Option<std::result::Result<sims_sim::pose::portrait::view::Input, String>>,
    #[cfg(test)]
    pub journey_tutorial_buttons: Option<sims_render::dialog::client::buttons::Buttons>,
    #[cfg(test)]
    pub journey_camera_watch: Option<JourneyCameraWatch>,
    #[cfg(test)]
    pub journey_ground: Option<std::result::Result<JourneyGround, String>>,
    #[cfg(test)]
    pub journey_walking: Option<std::result::Result<JourneyWalking, String>>,
    #[cfg(test)]
    pub journey_object: Option<std::result::Result<JourneyObject, String>>,
}
impl Published {
    pub fn button(&self, kind: Widget) -> Option<ButtonView> {
        self.controls
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, state)| *state)
    }
    pub fn capture(app: &Viewer) -> Self {
        let widgets = app.device.as_ref().and_then(|d| d.panel_widgets());
        let native_clock = app.native_clock();
        let native_transport = app.native_transport();
        let panel_state = app.device.as_ref().and_then(|d| d.panel_state());
        let people_visible = app.people_panel_visible();
        let pending = app.presentation_pending();
        let mut out = Self {
            window_observation: capture_window_observation(app),
            application: app.application.clone(),
            scene: app.scene.clone(),
            art: app.art.clone(),
            cluster: app.hud_cluster.clone(),
            widget_epoch: app.widget_epoch,
            title: "Original household".into(),
            session_present: app.session.is_some(),
            native_clock,
            native_transport,
            cursor_idle: app.device.as_ref().and_then(|d| d.tool_cursor()) == Some(0),
            native_speed: app
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.transport.as_ref())
                .map(|t| t.controls.speed),
            playing: app.playing,
            speed: app.speed,
            zoom: app.zoom,
            device_zoom: app.device.as_ref().map(|d| d.controls().zoom),
            pending,
            modal: app
                .session
                .as_ref()
                .is_some_and(|s| s.simulation.vm.world.dialogs.modal.is_some()),
            pause: app.device.as_ref().and_then(CameraDevice::transport_pause),
            paused_indicator: app.device.as_ref().is_some_and(|d| {
                d.mode_window()
                    .map_or(d.transport_pause().is_some_and(|v| v != 0), |w| {
                        w.indicator_visible()
                    })
            }),
            people_visible,
            panel_state,
            active_panel: widgets
                .and_then(|w| w.presentation())
                .and_then(|p| p.active()),
            root_visible: panel_state != Some(1)
                && widgets
                    .and_then(|w| w.presentation())
                    .is_none_or(|p| p.root_visible()),
            controls: widgets
                .map(|w| {
                    [
                        Widget::ZoomIn,
                        Widget::ZoomOut,
                        Widget::WallsDown,
                        Widget::WallsUp,
                        Widget::Cutaway,
                        Widget::Live,
                        Widget::Buy,
                        Widget::Build,
                        Widget::Options,
                        Widget::Camera,
                        Widget::Status,
                        Widget::FloorOne,
                        Widget::FloorTwo,
                        Widget::Roof,
                        Widget::Normal,
                        Widget::Fast,
                        Widget::Ultra,
                        Widget::Label,
                    ]
                    .into_iter()
                    .map(|kind| (kind, w.button(kind)))
                    .collect()
                })
                .unwrap_or_default(),
            pause_button: widgets.map(|w| w.pause()),
            can_buy: app
                .session
                .as_ref()
                .is_some_and(|s| s.host.presentation.tool_selection.is_some()),
            hud_height: app.hud_height(),
            tutorial_highlight: app.tutorial_highlight,
            status: app.feedback_status().to_owned(),
            save_load_feedback: app.retained_feedback().map(str::to_owned),
            player_shortcuts_blocked: app.player_shortcuts_blocked(),
            camera_shortcuts_blocked: app.camera_shortcuts_blocked(),
            load_picker_blocked: app.load_owner_blocked(),
            load_seed: app.load_seed().to_path_buf(),
            error: app.error.clone(),
            policy: app
                .session
                .as_ref()
                .map(|s| s.options.routing)
                .unwrap_or_default(),
            resident: app.resident,
            resident_selection: resident_selection::Snapshot::capture(app),
            residents: vec![],
            money: None,
            clock_fallback: None,
            funds: String::new(),
            needs: app.client.as_ref().and_then(|c| c.live_panel()).cloned(),
            legacy_motives: None,
            mood: None,
            menu: None,
            queue: vec![],
            catalog: widgets
                .and_then(|w| w.catalog())
                .map(|c| Catalog::capture(c, app)),
            furniture: app
                .device
                .as_ref()
                .and_then(|d| d.furniture_selection())
                .map(|s| Furniture {
                    name: s.name.clone(),
                    price: s.price,
                    sell: app.sell_message().map(|message| match message {
                        Message::Sell(id) => Intent::Sell(id),
                        _ => unreachable!("sale view produced another command"),
                    }),
                }),
            tick_label: None,
            dialog: None,
            native_popup: app
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.live_menu.clone()),
            native_popup_epoch: 0,
            #[cfg(test)]
            audit: None,
            #[cfg(test)]
            native_selection: app
                .session
                .as_ref()
                .map(|session| session.simulation.native_selected_person()),
            #[cfg(test)]
            journey_targets: capture_journey_targets(app),
            #[cfg(test)]
            journey_camera: app.session.as_ref().map(|session| {
                session
                    .retained_camera_input()
                    .map_err(|error| format!("{error:#}"))
            }),
            #[cfg(test)]
            journey_tutorial_buttons: app
                .client
                .as_ref()
                .and_then(|client| client.buttons())
                .cloned(),
            #[cfg(test)]
            journey_camera_watch: None,
            #[cfg(test)]
            journey_ground: None,
            #[cfg(test)]
            journey_walking: None,
            #[cfg(test)]
            journey_object: None,
        };
        let Some(s) = &app.session else {
            return out;
        };
        if let Some(h) = &s.house.household {
            out.title = h.name.clone();
            out.residents = h
                .residents
                .iter()
                .map(|r| Resident {
                    object: r.object as i16,
                    neighbor: r.neighbor,
                    name: r.name.clone(),
                    enabled: live_resident(s, r.object as i16).is_some()
                        && s.simulation.has_interaction_actor(r.object as i16),
                    highlighted: app
                        .client
                        .as_ref()
                        .and_then(|c| c.buttons())
                        .is_some_and(|b| b.highlighted(r.neighbor)),
                })
                .collect();
        }
        let texts = widgets.and_then(|w| w.texts());
        out.funds = household_funds_label(s, texts.map(|t| t.funds()).unwrap_or_default())
            .unwrap_or_default();
        out.money = texts.map(|texts| {
            let access = widgets.unwrap().access();
            Money {
                clock: texts.clock(),
                funds: household_funds_label(s, texts.funds())
                    .unwrap_or_else(|_| "Funds unavailable".into()),
                highlighted: widgets.unwrap().button(Widget::Status).selected,
                secondary: access
                    .filter(|a| a.label_visible())
                    .map(|a| a.secondary_text().to_owned()),
                funds_visible: access.is_none_or(|a| a.funds_visible()),
            }
        });
        out.clock_fallback = s.simulation.household.as_ref().map(|h| {
            format!(
                "Day {} · {:02}:{:02}",
                h.clock.elapsed_days + 1,
                h.clock.hour,
                h.clock.minute
            )
        });
        out.legacy_motives = s
            .simulation
            .vm
            .world
            .objects
            .get(&app.resident)
            .filter(|_| panel_state != Some(1))
            .filter(|_| live_resident(s, app.resident).is_some())
            .and_then(|o| o.motives)
            .map(|v| v.to_vec());
        out.mood = hud::live_gauge_mood(s).ok();
        out.menu = capture_menu(app);
        if live_resident(s, app.resident).is_some()
            && let Some(actor) = s
                .simulation
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|a| a.actors.get(&app.resident))
        {
            let resident_lifetime = s
                .simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|storage| storage.handle(app.resident));
            for (action, active) in actor
                .active
                .iter()
                .map(|action| (action, true))
                .chain(actor.queue.pending.iter().map(|action| (action, false)))
            {
                let image = s
                    .simulation
                    .vm
                    .world
                    .objects
                    .get(&action.icon_owner)
                    .and_then(|o| o.definition.as_ref())
                    .and_then(|d| app.thumbnails.as_ref()?.live_image(d))
                    .or_else(|| {
                        s.house
                            .household
                            .as_ref()?
                            .residents
                            .iter()
                            .find(|r| r.object as i16 == action.icon_owner)
                            .and_then(|r| app.art.as_ref()?.resident(r.neighbor as i16))
                    });
                let diagnostic_label = action
                    .label
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        super::super::support::entries(&s.program, &s.simulation, action.target)
                            .iter()
                            .find(|e| e.id == action.menu_id)
                            .map(|e| e.label.clone())
                    })
                    .unwrap_or_else(|| "Interaction".into());
                out.queue.push(QueueEntry {
                    action: QueueAction::capture(app.resident, resident_lifetime, action, active),
                    label: app.action_label(action),
                    diagnostic_label,
                    target: app.label(action.target),
                    image,
                });
            }
        }
        out.tick_label = Some(format!(
            "Tick {} (+{})",
            s.simulation.vm.tick,
            s.simulation.vm.tick - u64::from(s.house.simulator.ticks)
        ));
        let dialogs = &s.simulation.vm.world.dialogs;
        out.dialog = dialogs
            .modal
            .and_then(|id| dialogs.records.iter().find(|r| r.id == id))
            .or_else(|| dialogs.records.iter().find(|r| r.status == 1 && !r.modal()))
            .cloned();
        #[cfg(test)]
        {
            let owner = s.simulation.vm.world.ui_owner.owner;
            let object = owner.and_then(|owner| s.simulation.vm.world.objects.get(&owner));
            let wait = s.host.presentation.ui_wait.as_ref();
            out.journey_camera_watch = Some(JourneyCameraWatch {
                tick: s.simulation.vm.tick,
                lesson: out.dialog.clone(),
                camera: out
                    .journey_camera
                    .as_ref()
                    .and_then(|input| input.as_ref().ok())
                    .copied(),
                owner,
                owner_source: object
                    .and_then(|object| object.definition.as_ref())
                    .map(|definition| definition.source.clone()),
                query: object.map(|object| [object.temps[0], object.temps[1]]),
                baseline: wait.map(|wait| wait.baseline()),
                attempts: s
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .and_then(|presenter| presenter.camera.as_ref())
                    .map(|camera| camera.settings.attempts),
                poll_pending: wait.map(|wait| wait.pending()),
                world_rotation: s
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .and_then(|presenter| presenter.camera.as_ref())
                    .map(|camera| camera.settings.world_rotation),
                callback_pending: s.simulation.ui_wait_callback.is_some(),
            });
            if std::env::var("SIMS_HOST_JOURNEY_TUTORIAL_GO_HERE").as_deref() == Ok("1") {
                out.journey_ground =
                    Some(JourneyGround::capture(app).map_err(|error| format!("{error:#}")));
            }
            if [
                "SIMS_HOST_JOURNEY_TUTORIAL_CAMERA",
                "SIMS_HOST_JOURNEY_TUTORIAL_GO_HERE",
                "SIMS_HOST_JOURNEY_TUTORIAL_WALKING",
            ]
            .iter()
            .all(|name| std::env::var(name).as_deref() == Ok("1"))
            {
                out.journey_walking =
                    Some(JourneyWalking::capture(app).map_err(|error| format!("{error:#}")));
            }
            if std::env::var("SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_INTERACTION").as_deref() == Ok("1")
            {
                out.journey_object = Some(
                    JourneyObject::capture(app, &out.journey_targets)
                        .map_err(|error| format!("{error:#}")),
                );
            }
        }
        out
    }
}

fn capture_window_observation(app: &Viewer) -> Option<Arc<serde_json::Value>> {
    // A production registration exists only through this Recorder. The second
    // condition also lets UNRUN controls supply an isolated test ledger without
    // starting a global recorder or emitting fabricated journal records.
    if !window_session::enabled() && app.loaded_input.is_none() {
        return None;
    }
    if let Some(error) = app.error.as_deref().or(app.menu_error.as_deref()) {
        window_session::emit("application_error", serde_json::json!({"error":error}));
    }
    Some(Arc::new(window_observation_value(app)))
}

fn window_observation_value(app: &Viewer) -> serde_json::Value {
    let session = app.session.as_ref();
    let menu = capture_menu(app).map(|menu| {
        serde_json::json!({
            "title": menu.title,
            "preview": menu.preview,
            "ready": menu.ready,
            "visible": menu.popup_visible,
            "choices": menu.popup.iter().map(|choice| {
                serde_json::json!({"label":choice.label,"enabled":choice.intent.is_some()})
            }).collect::<Vec<_>>()
        })
    });
    let object_input = session
        .and_then(|session| session.host.presentation.object_input.as_ref())
        .map(|input| {
            serde_json::json!({"target":input.target(),"ready":input.ready(),"actions":input.actions()})
        });
    let ground_input = session
        .and_then(|session| session.host.presentation.ground_input.as_ref())
        .map(|input| serde_json::json!({"ready":input.ready(),"actions":input.actions()}));
    let actor = session
        .and_then(|session| session.simulation.vm.world.actions.as_ref())
        .and_then(|actions| actions.actors.get(&app.resident));
    let resident_position =
        session.and_then(|session| session.simulation.vm.world.positions.get(&app.resident));
    let ground_destination = session.and_then(|session| {
        session
            .host
            .presentation
            .ground_input
            .as_ref()
            .and_then(|input| input.target())
            .and_then(|target| session.simulation.vm.world.positions.get(&target))
    });
    serde_json::json!({
        "loaded_input": app.loaded_input_capture(),
        "tick": session.map(|session| session.simulation.vm.tick),
        "status": app.status,
        "save_feedback": app.save_feedback,
        "load_feedback": app.load_feedback,
        "application_mode": session.and_then(|session| session.host.presentation.presenter.as_ref())
            .and_then(|presenter| presenter.camera.as_ref())
            .map(|camera| camera.settings.application_mode),
        "people_visible": app.people_panel_visible(),
        "panel_state": app.device.as_ref().and_then(|device| device.panel_state()),
        "needs_selected": app.client.as_ref().and_then(|client| client.live_panel())
            .map(|panel| panel.selected()),
        "walls": session.and_then(|session| session.host.presentation.presenter.as_ref())
            .map(|presenter| serde_json::json!({
                "all_walls":presenter.view.floor.all_walls,
                "cutaway":presenter.view.floor.visibility.enabled
            })),
        "resident": app.resident,
        "selected": app.selected,
        "playing": app.playing,
        "pending": app.presentation_pending(),
        "clock_pending": session.is_some_and(Session::clock_pending),
        "presentation_children_pending": app.presentation_children_pending(),
        "clock": session.and_then(|session| session.host.presentation.clock.as_ref()).map(|clock| {
            serde_json::json!({"now":clock.now,"fields":clock.fields,
                "at_completed_update":clock.at_completed_update()})
        }),
        "hud_ready": app.hud_cluster.as_ref().map(|cluster| cluster.values.ready),
        "hud_transport_ready": serde_json::Value::Null,
        "hud_normal_ready": serde_json::Value::Null,
        "error": app.error,
        "menu_error": app.menu_error,
        "modal": session.is_some_and(|session| session.simulation.vm.world.dialogs.modal.is_some()),
        "dialog": session.and_then(|session| {
            let dialogs = &session.simulation.vm.world.dialogs;
            dialogs.records.iter().find(|record| Some(record.id) == dialogs.modal)
        }).map(window_observation::dialog),
        "dialog_answer": app.window_dialog_answer,
        "camera": session.and_then(|session| session.retained_camera_input().ok()),
        "scene_size": app.scene.as_ref().map(|scene| [scene.width,scene.height]),
        "scene_id": window_session::scene_id(app.scene.as_ref()),
        "display_size": app.display_size,
        "actor": actor,
        "actions": window_observation::actions(actor),
        "work": session.map(|session| window_observation::work(&session.simulation, app.resident)),
        "terminals": serde_json::Value::Null,
        "observation_capabilities": {
            "action_terminals":false,"route_terminals":false,"transport_readiness":false
        },
        "resident_position": resident_position,
        "ground_destination": ground_destination,
        "menu": menu,
        "object_input": object_input,
        "ground_input": ground_input
    })
}

fn capture_menu(app: &Viewer) -> Option<Menu> {
    let s = app.session.as_ref()?;
    let live = live_resident(s, app.resident).is_some();
    let action_labels = |actions: &[sims_vm::action::Action], ground: bool| {
        let diagnostic = actions
            .iter()
            .enumerate()
            .map(|(i, a)| Choice {
                label: s
                    .simulation
                    .vm
                    .action_menu(&s.program, a.target)
                    .ok()
                    .and_then(|table| table.iter().find(|e| e.id == a.menu_id))
                    .map(|e| e.label.clone())
                    .unwrap_or_default(),
                intent: Some(if ground {
                    Intent::GroundChoice(i)
                } else {
                    Intent::ObjectChoice(i)
                }),
            })
            .collect();
        let popup = actions
            .iter()
            .enumerate()
            .map(|(i, a)| Choice {
                label: app.action_label(a),
                intent: Some(if ground {
                    Intent::GroundChoice(i)
                } else {
                    Intent::ObjectChoice(i)
                }),
            })
            .collect();
        (diagnostic, popup)
    };
    if let Some(input) = &s.host.presentation.ground_input {
        let (diagnostic, popup) = action_labels(input.actions(), true);
        return Some(Menu {
            title: "Go here".into(),
            diagnostic_title: "Move here".into(),
            preview: false,
            ready: input.ready(),
            diagnostic,
            popup,
            cancel: input.can_cancel().then_some(Intent::GroundCancel),
            error: None,
            popup_visible: input.ready() || app.error.is_some(),
        });
    }
    if let Some(input) = &s.host.presentation.object_input {
        let (diagnostic, popup) = action_labels(input.actions(), false);
        return Some(Menu {
            title: app.label(input.target()),
            diagnostic_title: app.label(input.target()),
            preview: false,
            ready: input.ready(),
            diagnostic,
            popup,
            cancel: input.can_cancel().then_some(Intent::ObjectCancel),
            error: None,
            popup_visible: input.ready() || app.error.is_some(),
        });
    }
    app.selected.map(|id| Menu {
        title: app.label(id),
        diagnostic_title: app.label(id),
        preview: true,
        ready: true,
        diagnostic: app
            .choices
            .iter()
            .map(|c| Choice {
                label: c.label.clone(),
                intent: (c.available && live).then_some(Intent::Queue(id, c.index)),
            })
            .collect(),
        popup: app
            .choices
            .iter()
            .filter(|c| c.available)
            .map(|c| Choice {
                label: c.label.clone(),
                intent: Some(Intent::Queue(id, c.index)),
            })
            .collect(),
        cancel: Some(Intent::ClearSelection),
        error: app.menu_error.clone(),
        popup_visible: true,
    })
}

#[cfg(test)]
#[path = "ui_publication/tests.rs"]
mod tests;

#[cfg(test)]
mod journey_target_tests {
    use super::JourneyTarget;
    use sims_vm::{
        Definition, Module, Object, Program, World, multitile::Part, program::MenuEntry,
    };
    use std::collections::BTreeMap;

    fn fixture(labels: &[&str]) -> (World, Program) {
        let mut world = World::default();
        let mut module = Module::default();
        for (object, definition) in [(173, 1), (259, 2)] {
            world.objects.insert(
                object,
                Object {
                    definition: Some(Definition {
                        source: "journey-test".into(),
                        object: definition,
                        guid: 0,
                        slots: 0,
                    }),
                    ..Default::default()
                },
            );
            module
                .object_definition_words
                .insert(definition, vec![0; 64]);
            module.object_interactions.insert(definition, Vec::new());
        }
        module.object_interactions.insert(
            1,
            labels
                .iter()
                .map(|label| MenuEntry {
                    label: (*label).into(),
                    ..Default::default()
                })
                .collect(),
        );
        let program = Program {
            modules: BTreeMap::from([("journey-test".into(), module)]),
            ..Default::default()
        };
        (world, program)
    }

    fn redirect(world: &mut World, program: &mut Program) {
        let module = program.modules.get_mut("journey-test").unwrap();
        for (object, definition, index, head, next) in
            [(173, 1, 7, None, Some(259)), (259, 2, -7, Some(173), None)]
        {
            let words = module.object_definition_words.get_mut(&definition).unwrap();
            words[8] = index;
            words[10] = 1;
            world.objects.get_mut(&object).unwrap().multitile = Some(Part {
                head,
                next,
                origin: [0; 3],
                offset: [0; 3],
            });
        }
    }

    #[test]
    fn journey_target_hint_requires_exact_loaded_table_sit() {
        for (labels, expected) in [
            (vec![], false),
            (vec!["sit", "SIT", " Sit", "Sit ", "Sit Here"], false),
            (vec!["Stand", "Sit"], true),
        ] {
            let (world, program) = fixture(&labels);
            assert_eq!(
                JourneyTarget::capture(&world, &program, 173),
                Ok(JourneyTarget::Object {
                    target: 173,
                    table_sit: expected,
                })
            );
        }
    }

    #[test]
    fn journey_target_hint_uses_resolved_object_table_not_clicked_table() {
        let (mut world, mut program) = fixture(&["Sit"]);
        redirect(&mut world, &mut program);
        assert_eq!(
            JourneyTarget::capture(&world, &program, 173),
            Ok(JourneyTarget::Object {
                target: 259,
                table_sit: false,
            })
        );
        program
            .modules
            .get_mut("journey-test")
            .unwrap()
            .object_interactions
            .insert(
                2,
                vec![MenuEntry {
                    label: "Sit".into(),
                    ..Default::default()
                }],
            );
        assert_eq!(
            JourneyTarget::capture(&world, &program, 173),
            Ok(JourneyTarget::Object {
                target: 259,
                table_sit: true,
            })
        );
    }

    #[test]
    fn journey_target_hint_identifies_direct_and_redirected_visitors_without_a_table() {
        let (mut world, mut program) = fixture(&["Sit"]);
        world.objects.get_mut(&259).unwrap().person = Some(BTreeMap::new());
        program
            .modules
            .get_mut("journey-test")
            .unwrap()
            .object_interactions
            .remove(&2);
        assert_eq!(
            JourneyTarget::capture(&world, &program, 259),
            Ok(JourneyTarget::Person(259))
        );
        redirect(&mut world, &mut program);
        assert_eq!(
            JourneyTarget::capture(&world, &program, 173),
            Ok(JourneyTarget::Person(259))
        );
    }

    #[test]
    fn journey_target_hint_retains_resolution_and_missing_table_errors() {
        let (mut world, mut program) = fixture(&["Sit"]);
        assert_eq!(
            JourneyTarget::capture(&world, &program, 999),
            Err("interaction target missing".into())
        );
        program
            .modules
            .get_mut("journey-test")
            .unwrap()
            .object_interactions
            .remove(&1);
        assert_eq!(
            JourneyTarget::capture(&world, &program, 173),
            Err("resolved journey target interaction table unavailable".into())
        );
        redirect(&mut world, &mut program);
        world
            .objects
            .get_mut(&173)
            .unwrap()
            .multitile
            .as_mut()
            .unwrap()
            .next = Some(999);
        assert_eq!(
            JourneyTarget::capture(&world, &program, 173),
            Err("dangling multi-tile link".into())
        );
    }
}
