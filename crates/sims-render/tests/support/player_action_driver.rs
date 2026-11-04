//! Exploration adapter over actual Session + software Device services.
//! No native acknowledgements, input timing, RNG seeds or selected people are invented.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sims_content::Index;
use sims_render::{
    camera_device::Device,
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
};
use sims_runtime::{
    Session,
    exploration::{self, Candidate, Driver, Limits, Observation},
    original_session::{self, Progress, live_menu, move_tool},
};
use sims_vm::{ObjectId, action::Action};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub object: ObjectId,
    pub handle: u32,
}
impl Identity {
    pub fn capture(s: &Session, object: ObjectId) -> Result<Self> {
        Ok(Self {
            object,
            handle: s
                .simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.handle(object))
                .context("object lifetime absent")?,
        })
    }
    fn check(&self, s: &Session) -> Result<()> {
        ensure!(
            *self == Self::capture(s, self.object)?,
            "stale player object identity"
        );
        Ok(())
    }
}

/// An explicitly inspected original request, not a generic service ACK.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlEffectPin {
    pub program_sha256: String,
    pub caller: Identity,
    pub thread: u64,
    pub request: sims_vm::ui_effect::Request,
    pub continuation_sha256: String,
    pub resource_sha256: String,
    pub bhav_body_sha256: String,
}

/// The retained Dialog opcode constructor, before any presentation-host work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DialogConstructionPin {
    pub program_sha256: String,
    pub caller: Identity,
    pub thread: u64,
    pub request_id: u64,
    pub request_tick: u64,
    pub continuation_sha256: String,
    pub resource_sha256: String,
    pub bhav_body_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Select {
        identity: Identity,
    },
    OpenObject {
        identity: Identity,
    },
    Choose {
        index: usize,
        expected: Action,
    },
    ChooseFirst,
    CloseMenu,
    CancelLastQueued,
    BeginMode {
        mode: i32,
    },
    ExpandPanel {
        value: i32,
    },
    Pause {
        paused: bool,
    },
    Poll {
        budget: usize,
    },
    Drain {
        budget: usize,
    },
    GuardPendingInput,
    BuyCategory {
        label: String,
    },
    BuyProduct {
        source_contains: String,
    },
    EnterMove,
    Move {
        pointer: move_tool::motion::Input,
    },
    Button {
        input: move_tool::button::Input,
    },
    /// The native Cancel caller used by the app's Escape branch, after its
    /// higher-priority modal/menu owners have been ruled out. This exercises
    /// the handler's engine input; it is not an Iced keyboard-event test.
    EscapeMove {
        pointer: move_tool::motion::Input,
    },
    SellFurniture {
        held: u32,
    },
    ControlHighlight {
        pin: ControlEffectPin,
    },
    CheckMode {
        mode: i32,
    },
    CheckNoHeld,
    Step,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Boundary {
    Tick,
    PartialPass,
    Dialog,
    DialogConstruction(u64),
    UiEffect(u64),
    HouseNotification,
}

/// Positive tick credit requires BOTH the public completion result and its
/// actual +1 clock transition. A slice, partial pass or modal earns no credit.
pub fn classify(before: u64, after: u64, progress: Progress) -> Result<Boundary> {
    Ok(match progress {
        Progress::Tick => {
            ensure!(
                after == before.checked_add(1).context("tick overflow")?,
                "Tick result lacks exactly one completed clock advance"
            );
            Boundary::Tick
        }
        other => {
            ensure!(
                after == before,
                "non-Tick service advanced the completed-tick clock"
            );
            match other {
                Progress::PartialPass => Boundary::PartialPass,
                Progress::Dialog => Boundary::Dialog,
                Progress::DialogConstruction(id) => Boundary::DialogConstruction(id),
                Progress::UiEffect(id) => Boundary::UiEffect(id),
                Progress::HouseNotification => Boundary::HouseNotification,
                Progress::Tick => unreachable!(),
            }
        }
    })
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    session: original_session::Saved,
    camera_device: sims_render::camera_device::Saved,
    dialog_client: sims_render::dialog::client::Saved,
    #[serde(default)]
    last_queued: Option<(ObjectId, u32)>,
    #[serde(default)]
    last_boundary: Option<Boundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tutorial_control_highlight: Option<i32>,
}

pub struct Household {
    pub session: Session,
    pub device: Device,
    client: Client,
    index: Arc<Index>,
    last_queued: Option<(ObjectId, u32)>,
    last_boundary: Option<Boundary>,
    tutorial_control_highlight: Option<i32>,
    ticks: Option<Vec<Observation>>,
}
impl Household {
    #[allow(dead_code, reason = "shared with the fresh-source integration probe")]
    pub fn from_fresh_owners(
        index: Arc<Index>,
        session: Session,
        device: Device,
        client: Client,
    ) -> Self {
        Self {
            session,
            device,
            client,
            index,
            last_queued: None,
            last_boundary: None,
            tutorial_control_highlight: None,
            ticks: None,
        }
    }
    #[allow(dead_code, reason = "shared with the fresh-source integration probe")]
    pub fn bind_fresh_play_controls(&mut self) -> Result<()> {
        ensure!(
            self.session.host.presentation.move_tool.is_none(),
            "fresh play binding already has a Move owner"
        );
        bind_fresh_controls(&mut self.session, &mut self.device, &mut self.client)?;
        self.client.collect(&self.session)?;
        self.device.refresh(&self.session)
    }
    pub fn from_bytes(index: Arc<Index>, bytes: &[u8]) -> Result<Self> {
        let saved: Checkpoint = serde_json::from_slice(bytes)?;
        Self::from_checkpoint(index, saved)
    }
    #[allow(dead_code, reason = "shared with authored-notice continuation tests")]
    pub fn checkpoint_fixture(
        &self,
        store: &sims_fixtures::Store,
    ) -> Result<sims_fixtures::BlobRef> {
        store.put_value(&self.state(true)?)
    }
    #[allow(dead_code, reason = "shared with authored-notice continuation tests")]
    pub fn from_fixture(
        index: Arc<Index>,
        store: &sims_fixtures::Store,
        blob: &sims_fixtures::BlobRef,
    ) -> Result<Self> {
        Self::from_checkpoint(index, store.get_value(blob)?)
    }
    fn from_checkpoint(index: Arc<Index>, saved: Checkpoint) -> Result<Self> {
        ensure!(saved.session.version == 2, "managed input required");
        let session = Session::restore(&index, saved.session)?;
        let client = Client::restore_managed(&session, saved.dialog_client)?;
        let device = Device::restore(&session, saved.camera_device)?;
        Ok(Self {
            session,
            device,
            client,
            index,
            last_queued: saved.last_queued,
            last_boundary: saved.last_boundary,
            tutorial_control_highlight: saved.tutorial_control_highlight,
            ticks: None,
        })
    }
    fn state(&self, managed: bool) -> Result<Checkpoint> {
        Ok(Checkpoint {
            session: if managed {
                self.session.save_managed()?
            } else {
                self.session.save()
            },
            camera_device: self.device.save(&self.session)?,
            dialog_client: if managed {
                self.client.save_managed(&self.session)?
            } else {
                self.client.save(&self.session)?
            },
            last_queued: self.last_queued,
            last_boundary: self.last_boundary.clone(),
            tutorial_control_highlight: self.tutorial_control_highlight,
        })
    }
    pub fn research_state(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self.state(false)?)?)
    }
    #[allow(dead_code, reason = "shared with the continuation integration target")]
    pub fn collect_dialog_images(&mut self) -> Result<()> {
        self.client.collect(&self.session)
    }
    #[allow(dead_code, reason = "explicit retained dialog-construction host probe")]
    pub fn inspect_dialog_construction(&self, thread: u64) -> Result<DialogConstructionPin> {
        let vm = &self.session.simulation.vm;
        let request = vm.dialog_construction(thread).map_err(anyhow::Error::msg)?;
        let frame = request.frame();
        let resource =
            &self.session.index().resources[self.session.index().find(&frame.location.source)?];
        let routine = self
            .session
            .program
            .modules
            .get(&frame.location.source)
            .and_then(|m| m.routines.get(&frame.location.bhav))
            .context("dialog construction source BHAV absent")?;
        Ok(DialogConstructionPin {
            program_sha256: self.session.program.fingerprint(),
            caller: Identity::capture(&self.session, frame.caller)?,
            thread,
            request_id: request.id(),
            request_tick: request.tick(),
            continuation_sha256: exploration::digest_serialized(&vm.threads[&thread])?,
            resource_sha256: exploration::digest(&resource.read()?),
            bhav_body_sha256: exploration::digest_serialized(&routine.body)?,
        })
    }
    #[allow(
        dead_code,
        reason = "same host entry as original_viewer DialogConstruction"
    )]
    pub fn begin_dialog_construction(&mut self, pin: &DialogConstructionPin) -> Result<()> {
        ensure!(
            self.inspect_dialog_construction(pin.thread)? == *pin,
            "pinned dialog construction owner/request/source changed"
        );
        pin.caller.check(&self.session)?;
        ensure!(
            self.session
                .simulation
                .vm
                .object_threads
                .get(&pin.caller.object)
                == Some(&pin.thread),
            "dialog constructor caller is not its registered object thread"
        );
        let camera = OriginalHouse::native_camera(&self.session)?;
        let size = self
            .session
            .simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("dialog constructor lot absent")?
            .size as i32;
        self.client
            .begin(&self.session, pin.thread, Inputs::camera(camera, size)?)
    }
    #[allow(dead_code, reason = "same pending-client phase as original_viewer")]
    pub fn advance_dialog_construction(&mut self, budget: usize) -> Result<bool> {
        ensure!(
            self.client.pending(),
            "dialog constructor host is not pending"
        );
        self.client
            .advance(&mut self.session, budget)
            .map(|(complete, _)| complete)
    }
    #[allow(dead_code, reason = "explicit finite tutorial-host lane")]
    pub fn inspect_control_effect(&self, thread: u64) -> Result<ControlEffectPin> {
        let vm = &self.session.simulation.vm;
        let t = vm.threads.get(&thread).context("UI-effect thread absent")?;
        let sims_vm::Status::AwaitingUiEffect(call) = &t.status else {
            bail!("UI-effect call absent")
        };
        let frame = t.stack.last().context("UI-effect frame absent")?;
        let resource =
            &self.session.index().resources[self.session.index().find(&frame.location.source)?];
        let routine = self
            .session
            .program
            .modules
            .get(&frame.location.source)
            .and_then(|m| m.routines.get(&frame.location.bhav))
            .context("UI-effect source BHAV absent")?;
        Ok(ControlEffectPin {
            program_sha256: self.session.program.fingerprint(),
            caller: Identity::capture(&self.session, frame.caller)?,
            thread,
            request: call.request(),
            continuation_sha256: exploration::digest_serialized(t)?,
            resource_sha256: exploration::digest(&resource.read()?),
            bhav_body_sha256: exploration::digest_serialized(&routine.body)?,
        })
    }
    #[allow(dead_code, reason = "explicit finite tutorial-host lane")]
    pub fn complete_control_highlight(&mut self, pin: &ControlEffectPin) -> Result<()> {
        ensure!(
            self.inspect_control_effect(pin.thread)? == *pin,
            "pinned UI-effect owner/request/source changed"
        );
        ensure!(
            pin.request.kind == sims_vm::ui_effect::Kind::Control,
            "only an explicit Control effect is supported"
        );
        pin.caller.check(&self.session)?;
        ensure!(
            self.session
                .simulation
                .vm
                .object_threads
                .get(&pin.caller.object)
                == Some(&pin.thread),
            "UI-effect caller is not the registered object thread"
        );
        // Same frontend owners and control list as original_viewer.rs's
        // Progress::UiEffect branch. No UI-wait observation/button activation
        // is supplied by highlighting a control.
        let mut ids = vec![2011, 2012, 2013, 2014, 2023, 2024];
        if self.session.host.presentation.control_panel.is_some() {
            ids.push(2010)
        }
        if self.session.host.presentation.tool_selection.is_some() {
            ids.push(2008)
        }
        if self.client.live_panel().is_some() {
            ids.push(4500)
        }
        require_bound_control(&ids, i32::from(pin.request.value))?;
        self.tutorial_control_highlight =
            pin.request.enabled.then_some(i32::from(pin.request.value));
        self.session
            .simulation
            .vm
            .complete_ui_effect(&self.session.program, pin.thread, pin.request)
            .map_err(anyhow::Error::msg)
    }
    #[allow(dead_code, reason = "explicit finite tutorial Portrait second phase")]
    pub fn complete_absent_portrait_highlight(&mut self, pin: &ControlEffectPin) -> Result<()> {
        ensure!(
            self.inspect_control_effect(pin.thread)? == *pin,
            "pinned Portrait owner/request/source changed"
        );
        ensure!(
            pin.request.kind == sims_vm::ui_effect::Kind::Portrait,
            "explicit Portrait phase required"
        );
        pin.caller.check(&self.session)?;
        ensure!(
            self.session
                .simulation
                .vm
                .object_threads
                .get(&pin.caller.object)
                == Some(&pin.thread),
            "Portrait caller is not its registered object thread"
        );
        ensure!(
            sims_vm::ui_effect::neighbor_guid(&self.session.simulation.vm.world, pin.request.value)
                .map_err(anyhow::Error::msg)?
                .is_none(),
            "present neighbor requires a separately supported actual image host"
        );
        ensure!(
            self.client.buttons().is_some_and(|b| b.initialized),
            "Portrait buttons are not bound"
        );
        self.device.refresh(&self.session)?;
        let size = self
            .session
            .simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("Portrait architecture absent")?
            .size as i32;
        let inputs = sims_render::dialog::client::Inputs::camera(
            sims_render::original_house::OriginalHouse::native_camera(&self.session)?,
            size,
        )?;
        self.client
            .begin_portrait_highlight(&mut self.session, pin.thread, pin.request, inputs)?;
        for _ in 0..10000 {
            if !self.client.ui_lookup_pending() {
                break;
            }
            self.client.advance_ui_lookup(&mut self.session, 256)?;
        }
        ensure!(
            !self.client.ui_lookup_pending(),
            "Portrait host lookup budget exceeded"
        );
        Ok(())
    }
    /// Same automatic application poll as original_viewer::poll_application_ui.
    /// Unsupported observations remain explicit boundaries; highlighting or
    /// selecting a person never invents an activated resident/control button.
    #[allow(dead_code, reason = "UserData actual frontend-order probes")]
    pub fn poll_application_ui(&mut self) -> Result<bool> {
        use sims_runtime::original_session::ui_wait::Progress;
        use sims_vm::ui_owner::wait::{Condition, Observation};
        if self.client.ui_lookup_pending()
            && !self.client.advance_ui_lookup(&mut self.session, 256)?
        {
            return Ok(false);
        }
        if self.session.player_input_blocks_ticks() {
            return Ok(false);
        }
        if !self.session.ui_wait_pending() {
            let focused = self.client.buttons().and_then(|b| b.focused_resource());
            self.session.begin_ui_wait(focused)?;
        }
        for _ in 0..16 {
            match self.session.advance_ui_wait(256)? {
                Progress::Complete => return Ok(true),
                Progress::Pending => {}
                Progress::Observe(condition) => {
                    if let Condition::Control(id) = condition {
                        use sims_vm::interface::view_control::Widget;
                        let kind = match id {
                            2008 => Some(Widget::Buy),
                            2010 => Some(Widget::Live),
                            _ => None,
                        };
                        if let Some(state) =
                            kind.and_then(|k| self.device.panel_widgets().map(|w| w.button(k)))
                        {
                            self.session.acknowledge_ui_wait(
                                &condition,
                                Observation::Control {
                                    image: Some(id),
                                    state: u32::from(state.selected),
                                },
                            )?;
                            continue;
                        }
                    }
                    if let Some(observation) = self
                        .client
                        .live_panel()
                        .and_then(|p| p.observation(&condition))
                    {
                        self.session.acknowledge_ui_wait(&condition, observation)?;
                        continue;
                    }
                    if let Condition::Portrait(neighbor) = condition
                        && self.client.buttons().is_some()
                    {
                        let size = self
                            .session
                            .simulation
                            .architecture
                            .as_ref()
                            .and_then(|a| a.original.as_ref())
                            .context("UI wait portrait geometry absent")?
                            .size as i32;
                        let inputs = sims_render::dialog::client::Inputs::camera(
                            sims_render::original_house::OriginalHouse::native_camera(
                                &self.session,
                            )?,
                            size,
                        )?;
                        self.client.begin_portrait_observation(
                            &mut self.session,
                            neighbor,
                            inputs,
                        )?;
                        if self.client.advance_ui_lookup(&mut self.session, 256)? {
                            continue;
                        }
                        return Ok(false);
                    }
                    bail!("original UI wait requires its actual widget owner: {condition:?}");
                }
            }
        }
        Ok(false)
    }
    #[allow(dead_code, reason = "shared with the continuation integration target")]
    pub fn queued_receipt(&self) -> Option<(ObjectId, u32)> {
        self.last_queued
    }
    pub fn mode(&self) -> Result<i32> {
        Ok(self
            .session
            .host
            .presentation
            .transport
            .as_ref()
            .context("CPState absent")?
            .controls
            .mode)
    }
    fn pending(&self) -> bool {
        let s = &self.session;
        s.publication_pending()
            || s.tool_selection_pending()
            || s.move_tool_pending()
            || s.move_button_pending()
            || s.transport_pending()
            || s.camera_command_pending()
            || s.control_panel_pending()
            || s.interface_update_pending()
            || s.house_notification_pending()
            || s.simulation.selection_work_pending()
            || s.host
                .presentation
                .object_input
                .as_ref()
                .is_some_and(|o| !o.ready())
    }
    fn poll(&mut self, budget: usize) -> Result<bool> {
        ensure!((1..=100_000).contains(&budget), "invalid service budget");
        let s = &mut self.session;
        let d = &mut self.device;
        if s.host
            .presentation
            .property_refresh
            .as_ref()
            .and_then(|p| p.panel_owner())
            .is_some()
        {
            ensure!(
                d.publish_replacement_people(s, &mut self.client)?,
                "property refresh requires an unsupported panel service"
            );
        } else if s.publication_pending() {
            match d.advance_publication(s, budget)? {
                original_session::publication::Progress::Complete
                | original_session::publication::Progress::Pending => (),
                original_session::publication::Progress::Panels(_) => ensure!(
                    d.publish_replacement_people(s, &mut self.client)?,
                    "publication requires an unsupported panel service"
                ),
                other => bail!("publication retained unsupported service: {other:?}"),
            }
        } else if s.move_button_pending() {
            d.advance_move_button(s, budget)?;
        } else if s.tool_selection_pending() {
            if d.advance_tool_selection(s, budget)? {
                s.republish_interface()?;
            }
        } else if s.move_tool_pending() {
            d.advance_move_tool(s, budget)?;
        } else if s.transport_pending() {
            if d.advance_transport(s, budget)? {
                s.republish_interface()?;
            }
        } else if s.control_panel_pending() {
            d.advance_panel(s)?;
        } else if s.interface_update_pending() {
            d.advance_interface(s, budget)?;
        } else if s.camera_command_pending() {
            d.advance(s, budget)?;
        } else if s.house_notification_pending() {
            d.advance_notification_with_client(s, &mut self.client, budget)?;
        } else if s.simulation.selection_work_pending() {
            let camera = s
                .host
                .presentation
                .camera
                .context("selection camera input absent")?;
            s.advance_selection(camera, budget)?;
        } else if s
            .host
            .presentation
            .object_input
            .as_ref()
            .is_some_and(|o| !o.ready())
        {
            s.advance_object_input(budget)?;
        } else {
            ensure!(
                !s.clock_pending() && !s.ui_portrait_pending() && !s.ui_wait_pending(),
                "retained caller requires an explicit service outside this driver"
            );
            if s.host
                .presentation
                .object_input
                .as_ref()
                .is_some_and(|o| o.ready())
                && !s.running_menu()
            {
                s.begin_running_menu()?;
            }
            // The real frontend publishes the ready surface after its input
            // caller returns. A cold pending device legitimately keeps its
            // last image frozen until this ordinary publication.
            d.refresh(s)?;
            return Ok(true);
        }
        Ok(false)
    }
    fn choose(&mut self, index: usize, expected: &Action) -> Result<()> {
        let s = &mut self.session;
        let owner = s
            .host
            .presentation
            .object_input
            .as_ref()
            .context("object menu absent")?;
        ensure!(
            owner.ready() && owner.actions().get(index) == Some(expected),
            "stale menu entry"
        );
        let actor = owner.actor();
        let (id, accepted) = if s.running_menu() {
            s.submit_menu_command(live_menu::Command::Choose(index))?;
            match s.dispatch_menu_command()? {
                Some(live_menu::Outcome::Queued { id, accepted }) => (id, accepted),
                other => bail!("menu return retained a service: {other:?}"),
            }
        } else {
            s.choose_object_action(index)?
        };
        // Native queue refusal is an ordinary return value, distinct from an
        // invalid input or a failed service. Only accepted entries can cancel.
        self.last_queued = accepted.then_some((actor, id));
        Ok(())
    }
}

#[allow(dead_code, reason = "also used by portable host ownership tests")]
pub fn require_bound_control(ids: &[i32], target: i32) -> Result<()> {
    use sims_vm::ui_effect::widgets::{Bitmap, Kind, Node, Query};
    ensure!(
        ids.contains(&target),
        "original UI control is not bound in this host"
    );
    let mut nodes = vec![Node {
        kind: Kind::Window,
        bitmap: None,
        children: (1..=ids.len()).collect(),
    }];
    nodes.extend(ids.iter().map(|&id| Node {
        kind: Kind::Button,
        bitmap: Some(Bitmap {
            handle: id as u32,
            resource: id,
        }),
        children: vec![],
    }));
    ensure!(
        sims_vm::ui_effect::widgets::find(&nodes, Some(0), Query::Control(target))
            .map_err(anyhow::Error::msg)?
            .is_some(),
        "bound control not found by actual native widget traversal"
    );
    Ok(())
}

impl Driver for Household {
    type Command = Command;
    fn observe(&self) -> Result<Observation> {
        Ok(Observation {
            tick: self.session.simulation.vm.tick,
            rng: [
                Some(self.session.simulation.vm.rng_state),
                self.session.simulation.vm.lot_rng_state,
                self.session.simulation.vm.crt_rng_state,
            ],
            state_sha256: exploration::digest_serialized(&self.state(false)?)?,
        })
    }
    fn candidates(&self) -> Result<Vec<Candidate<Command>>> {
        let add = |command, weight| Candidate { command, weight };
        if self.pending() {
            return Ok(vec![add(Command::Drain { budget: 256 }, 1)]);
        }
        if let Some(menu) = self
            .session
            .host
            .presentation
            .object_input
            .as_ref()
            .filter(|o| o.ready())
        {
            let mut out = vec![add(Command::CloseMenu, 2)];
            out.extend(
                menu.actions()
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.flags & 16 == 0)
                    .map(|(index, a)| {
                        add(
                            Command::Choose {
                                index,
                                expected: a.clone(),
                            },
                            2,
                        )
                    }),
            );
            return Ok(out);
        }
        let mut out = vec![add(
            Command::BeginMode {
                mode: if self.mode()? == 2 { 1 } else { 2 },
            },
            2,
        )];
        if self.mode()? == 2 {
            if self.last_queued.is_some() {
                out.push(add(Command::CancelLastQueued, 2));
            }
            let selected = self
                .session
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?;
            let ids: std::collections::BTreeSet<_> = self
                .device
                .scene()
                .draws
                .iter()
                .filter_map(|d| d.object)
                .collect();
            for id in &ids {
                if Some(*id) != selected
                    && self
                        .session
                        .simulation
                        .vm
                        .world
                        .objects
                        .get(id)
                        .is_some_and(|o| o.person.is_some())
                {
                    out.push(add(
                        Command::Select {
                            identity: Identity::capture(&self.session, *id)?,
                        },
                        2,
                    ));
                }
            }
            if selected.is_some() {
                for id in ids
                    .into_iter()
                    .filter(|id| {
                        self.session
                            .simulation
                            .vm
                            .world
                            .objects
                            .get(id)
                            .is_some_and(|o| {
                                o.person.is_none() && o.cursor.is_none() && o.definition.is_some()
                            })
                    })
                    .take(12)
                {
                    out.push(add(
                        Command::OpenObject {
                            identity: Identity::capture(&self.session, id)?,
                        },
                        1,
                    ));
                }
            }
        }
        Ok(out)
    }
    fn execute(&mut self, command: &Command, limits: &Limits) -> Result<()> {
        match command {
            Command::Select { identity } => {
                identity.check(&self.session)?;
                self.session.begin_selection(Some(identity.object))?;
            }
            Command::OpenObject { identity } => {
                identity.check(&self.session)?;
                ensure!(
                    self.session.begin_object_input(identity.object)?,
                    "object click had no selected person"
                );
            }
            Command::Choose { index, expected } => self.choose(*index, expected)?,
            Command::ChooseFirst => {
                let (index, action) = self
                    .session
                    .host
                    .presentation
                    .object_input
                    .as_ref()
                    .filter(|o| o.ready())
                    .context("object menu absent")?
                    .actions()
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.flags & 16 == 0)
                    .map(|(i, a)| (i, a.clone()))
                    .context("no offered enabled action")?;
                self.choose(index, &action)?;
            }
            Command::CloseMenu => {
                if self.session.running_menu() {
                    self.session
                        .submit_menu_command(live_menu::Command::Cancel)?;
                    ensure!(
                        self.session.dispatch_menu_command()?
                            == Some(live_menu::Outcome::Cancelled),
                        "menu cancellation retained a service"
                    );
                } else {
                    self.session.cancel_object_input()?;
                }
            }
            Command::CancelLastQueued => {
                let (actor, id) = self.last_queued.context("no queued receipt")?;
                ensure!(
                    self.session
                        .simulation
                        .vm
                        .cancel_action(&self.session.program, actor, id)
                        .map_err(anyhow::Error::msg)?,
                    "queued receipt no longer present"
                );
                self.last_queued = None;
            }
            Command::BeginMode { mode } => {
                ensure!(
                    matches!(mode, 1 | 2),
                    "only declared Buy/Live mode inputs supported"
                );
                self.session.begin_mode_change(*mode, 0)?;
            }
            Command::ExpandPanel { value } => {
                self.device
                    .begin_panel_expansion(&mut self.session, *value)?;
            }
            Command::Pause { paused } => {
                self.session.begin_pause(*paused)?;
            }
            Command::Poll { budget } => {
                self.poll(*budget)?;
            }
            Command::Drain { budget } => {
                let mut complete = false;
                for _ in 0..limits.service_polls {
                    if self.poll(*budget)? {
                        complete = true;
                        break;
                    }
                }
                ensure!(complete, "retained input service poll budget exhausted");
            }
            Command::GuardPendingInput => {
                ensure!(
                    self.session.player_input_blocks_ticks(),
                    "no blocking input owner to guard"
                );
                let before = self.observe()?;
                ensure!(
                    self.session.step().is_err(),
                    "pending input permitted an ordinary tick"
                );
                ensure!(
                    self.session.begin_mode_change(1, 0).is_err(),
                    "pending input permitted mode reentry"
                );
                ensure!(
                    self.session.begin_selection(None).is_err(),
                    "pending input permitted selection reentry"
                );
                ensure!(
                    self.observe()? == before,
                    "rejected pending input changed retained state"
                );
            }
            Command::BuyCategory { label } => {
                ensure!(
                    self.device
                        .panel_widgets()
                        .and_then(|w| w.presentation())
                        .is_some_and(|p| p.visible(sims_render::camera_device::panels::Panel::Buy)),
                    "authored category input has no visible expanded Buy parent"
                );
                let catalog = self
                    .device
                    .panel_widgets()
                    .and_then(|w| w.catalog())
                    .context("Buy catalog absent")?;
                let view = catalog.view().context("Buy catalog unpublished")?;
                let category = view
                    .labels
                    .iter()
                    .enumerate()
                    .find(|(i, actual)| {
                        actual.eq_ignore_ascii_case(label)
                            && view.categories[*i].visible
                            && view.categories[*i].enabled
                    })
                    .map(|(i, _)| i as u8)
                    .with_context(|| {
                        format!(
                            "authored Buy category absent or disabled; visible labels: {:?}",
                            view.labels
                        )
                    })?;
                ensure!(
                    self.device.begin_buy_navigation(
                        &mut self.session,
                        catalog.revision(),
                        sims_vm::interface::buy::input::Event::Category(category)
                    )?,
                    "published Buy category rejected navigation"
                );
            }
            Command::BuyProduct { source_contains } => {
                let c = self
                    .device
                    .panel_widgets()
                    .and_then(|w| w.catalog())
                    .context("Buy catalog absent")?;
                let index = c
                    .view()
                    .context("Buy catalog unpublished")?
                    .products
                    .iter()
                    .copied()
                    .find(|&i| {
                        c.product(i).is_some_and(|p| {
                            p.source
                                .source
                                .to_ascii_lowercase()
                                .contains(&source_contains.to_ascii_lowercase())
                        })
                    })
                    .with_context(|| {
                        format!(
                            "requested actual Buy product absent; first visible definitions: {:?}",
                            c.view().map(|view| view
                                .products
                                .iter()
                                .take(8)
                                .filter_map(|&row| c.product(row))
                                .map(|p| (&p.source, p.fields.group, p.fields.subindex))
                                .collect::<Vec<_>>())
                        )
                    })?;
                let revision = c.revision();
                ensure!(
                    self.device
                        .begin_buy_product(&mut self.session, revision, index)?,
                    "actual Buy item rejected"
                );
            }
            Command::EnterMove => {
                self.session.begin_move_tool_viewport_enter()?;
            }
            Command::Move { pointer } => {
                self.session.begin_move_tool_motion(*pointer)?;
            }
            Command::Button { input } => {
                self.session.begin_move_button(*input)?;
            }
            Command::EscapeMove { pointer } => {
                ensure!(
                    self.session.simulation.vm.world.dialogs.modal.is_none()
                        && !self
                            .session
                            .host
                            .presentation
                            .ground_input
                            .as_ref()
                            .is_some_and(|i| i.can_cancel())
                        && !self
                            .session
                            .host
                            .presentation
                            .object_input
                            .as_ref()
                            .is_some_and(|i| i.can_cancel()),
                    "Escape belongs to a higher-priority modal/menu owner"
                );
                self.session.begin_move_button(move_tool::button::Input {
                    kind: move_tool::button::Kind::Cancel,
                    pointer: *pointer,
                    shift: false,
                })?;
            }
            Command::SellFurniture { held } => {
                ensure!(
                    self.session
                        .host
                        .presentation
                        .move_tool
                        .as_ref()
                        .is_some_and(|m| m.tool.held == Some(*held))
                        && self
                            .device
                            .furniture_selection()
                            .is_some_and(|s| s.sell_allowed),
                    "stale or unavailable actual sale selection"
                );
                self.session.begin_move_tool_sale()?;
            }
            Command::ControlHighlight { pin } => self.complete_control_highlight(pin)?,
            Command::CheckMode { mode } => {
                ensure!(self.mode()? == *mode && !self.pending(), "mode not ready");
            }
            Command::CheckNoHeld => {
                ensure!(
                    self.session
                        .host
                        .presentation
                        .move_tool
                        .as_ref()
                        .is_some_and(|m| m.tool.held.is_none()),
                    "Move still holds furniture"
                );
            }
            Command::Step => {
                let before = self.session.simulation.vm.tick;
                let result = self.session.step()?;
                let boundary = classify(before, self.session.simulation.vm.tick, result)?;
                if matches!(boundary, Boundary::Tick | Boundary::PartialPass) {
                    self.client.collect(&self.session)?;
                    if boundary == Boundary::Tick {
                        self.device.refresh(&self.session)?;
                    }
                }
                self.last_boundary = Some(boundary.clone());
                if boundary == Boundary::Tick && self.ticks.is_some() {
                    let value = self.observe()?;
                    if let Some(ticks) = &mut self.ticks {
                        ticks.push(value);
                    }
                }
            }
        }
        Ok(())
    }
    fn begin_tick_trace(&mut self) {
        self.ticks = Some(vec![]);
    }
    fn take_tick_trace(&mut self) -> Vec<Observation> {
        self.ticks.take().unwrap_or_default()
    }
    fn fork(&self) -> Result<Self> {
        let session = self.session.fork_for_exploration()?;
        let client = Client::restore(&session, self.client.save(&self.session)?)?;
        Ok(Self {
            session,
            device: self.device.fork_for_exploration(),
            client,
            index: self.index.clone(),
            last_queued: self.last_queued,
            last_boundary: self.last_boundary.clone(),
            tutorial_control_highlight: self.tutorial_control_highlight,
            ticks: None,
        })
    }
    fn checkpoint(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&self.state(true)?)?)
    }
    fn restore(&self, bytes: &[u8]) -> Result<Self> {
        Self::from_bytes(self.index.clone(), bytes)
    }
}

// Explicit replacement-frontend IDs and API order from original_viewer_play::bind.
// These are new application owners, not reconstructed Windows addresses.
fn bind_fresh_controls(
    session: &mut Session,
    device: &mut Device,
    client: &mut Client,
) -> Result<()> {
    use sims_render::camera_device::{
        mode::Window,
        panels::{Owners, Panel},
    };
    use sims_runtime::original_session::publication::{Context, Progress};
    use sims_runtime::original_session::{
        move_tool,
        tool_selection::{self, Receiver},
    };
    use sims_sim::object_catalog::move_tool::{Groups, cleanup};

    ensure!(
        session.simulation.native_scheduler.is_some(),
        "Buy requires original partial scheduling"
    );
    session.bind_person_publication(0)?;
    let mut catalog = sims_content::object_catalog::Catalog::household(
        Arc::new(session.index().clone()),
        &session.house.neighborhood_source,
        Default::default(),
    )?;
    let mut history = move_tool::History::new(1);
    history.retain_catalog(&mut catalog)?;
    history
        .store
        .construct_manager(5001)
        .map_err(anyhow::Error::msg)?;
    let mut groups = Groups::default();
    catalog
        .populate_move_tool_groups(&mut groups)
        .map_err(anyhow::Error::msg)?;
    let mut moving = move_tool::State::new(
        cleanup::State::constructor_fields(5001),
        cleanup::Inputs {
            registry: 5002,
            window: 0,
            key_function: 0,
            control_state: 0,
        },
        history,
    );
    moving.retain_activation_groups(groups)?;
    session.bind_move_tool(moving)?;
    let mut slots = [None; 12];
    slots[6] = Some(6000);
    slots[7] = Some(7000);
    let mut tools = tool_selection::State::new(
        slots,
        [
            (6000, Receiver::Picker { mode: Some(0) }),
            (7000, Receiver::Move),
        ]
        .into(),
        sims_vm::interface::tool::Fields {
            active: Some(6000),
            argument: 0,
        },
    );
    tools.bind_mode_context(tool_selection::mode::Context::new(8000).with_history_manager(5001))?;
    session.bind_tool_selection(tools)?;
    session.bind_current_tools(Some(6000))?;
    let controls = &session
        .host
        .presentation
        .transport
        .as_ref()
        .context("transport absent")?
        .controls;
    device.bind_mode_window(
        session,
        Window::new(9000, Some(9001), 80, controls.paused, 0),
    )?;
    if session
        .host
        .presentation
        .presenter
        .as_ref()
        .and_then(|p| p.camera.as_ref())
        .is_some_and(|c| c.settings.roof.is_none())
    {
        device.bind_hidden_roof(session)?;
    }
    // 4ce81c samples the actual view with this precedence. A fresh Live view
    // normally has cutaway enabled; publishing "walls up" would change it.
    let presenter = session
        .host
        .presentation
        .presenter
        .as_ref()
        .context("view absent")?;
    let walls = if presenter.view.floor.all_walls != 0 {
        2
    } else if presenter.view.floor.visibility.enabled != 0 {
        4
    } else if presenter
        .camera
        .as_ref()
        .and_then(|c| c.settings.roof)
        .context("roof absent")?
        != 0
    {
        8
    } else {
        1
    };
    session.begin_interface_publication(
        Context {
            panel_owner: Some(10000),
            closing: 0,
            walls,
            tool_gate: 0,
            current_tool: Some(6000),
            selected_person_index: 0,
        },
        0,
    )?;
    for _ in 0..10000 {
        match device.advance_publication(session, 256)? {
            Progress::Complete => {
                session.bind_frame_transport()?;
                return Ok(());
            }
            Progress::Pending => (),
            Progress::Panels(_) if device.panel_widgets().is_none() => {
                device.bind_panel_widgets(
                    session,
                    Owners {
                        panel: 10000,
                        state: 10001,
                        controls: 10002,
                        lot: 10003,
                        manager: 10004,
                        panels: std::array::from_fn(|i| 10010 + i as u32),
                        widgets: std::array::from_fn(|i| 10100 + i as u32),
                        pause: 10200,
                    },
                    1,
                )?;
                device.bind_panel_presentation(session, Panel::People)?;
                device.bind_panel_text(session)?;
                device.bind_panel_access(session)?;
                device.bind_panel_catalog(session, 8)?;
            }
            Progress::Panels(_) if device.publish_replacement_people(session, client)? => (),
            other => anyhow::bail!(
                "play controls require unsupported startup service: {other:?}, {:?}",
                device.panel_widgets().and_then(|w| w.boundary())
            ),
        }
    }
    anyhow::bail!("play controls startup budget exceeded")
}
