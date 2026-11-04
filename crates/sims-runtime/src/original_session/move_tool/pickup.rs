//!486960: wrap an existing object, retain its grab offset, classify its
//!definition, then capture a kind1 undo command. No furniture is purchased.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///Replacement forward projection; physical row/column and current floor.
    Project { position: [i32; 2], level: i32 },
    ///487d7a/4cf90b: actual replacement selected-furniture controls.
    Select {
        object: u32,
        definition: u32,
        price: i32,
        move_allowed: bool,
        sell_allowed: bool,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Projected([i16; 2]),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Factory,
    Project,
    Classify,
    Capture,
    Tail,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Execution {
    input: motion::Input,
    tool: u32,
    phase: Phase,
    level: i32,
    definition: Option<sims_vm::Definition>,
    pending: Option<Request>,
}
impl Execution {
    pub(super) fn new(tool: u32, input: motion::Input) -> Self {
        Self {
            input,
            tool,
            phase: Phase::Entry,
            level: 0,
            definition: None,
            pending: None,
        }
    }
    pub(super) fn complete(&self) -> bool {
        self.phase == Phase::Complete && self.pending.is_none()
    }
    pub(super) fn owns_factory(&self) -> bool {
        self.phase == Phase::Factory
    }
    pub(super) fn validate(&self, s: &Session, state: &State) -> Result<()> {
        ensure!(
            state
                .preview_fields
                .as_ref()
                .is_some_and(|f| f.budget == self.tool),
            "pickup lost tool fields"
        );
        ensure!(
            self.owns_factory() == s.simulation.cursor_factory.is_some(),
            "pickup cursor factory ownership differs"
        );
        if let Some(f) = &s.simulation.cursor_factory {
            ensure!(
                f.matches_creation(self.definition.as_ref(), self.input.picked, self.level),
                "pickup factory inputs changed"
            );
        }
        ensure!(
            self.pending.is_none() || self.phase == Phase::Classify,
            "pickup projection lost continuation"
        );
        if let Some(Request::Project { position, level }) = self.pending {
            let held = id(
                &s.simulation.vm.world,
                state.tool.held.context("pickup projected cursor absent")?,
            )
            .map_err(anyhow::Error::msg)?;
            let p = &s.simulation.vm.world.positions[&held];
            ensure!(
                position == [p.x, p.y] && level == current_floor(s)?,
                "pickup projection changed"
            );
        }
        Ok(())
    }
    pub(super) fn acknowledge(
        &mut self,
        state: &mut State,
        r: Request,
        reply: Reply,
    ) -> Result<()> {
        ensure!(
            self.pending == Some(r),
            "pickup projection reply has no caller"
        );
        match (r, reply) {
            (Request::Project { .. }, Reply::Projected(point)) => {
                state.tool.tool.position = std::array::from_fn(|i| {
                    i32::from(self.input.screen[i]).wrapping_sub(i32::from(point[i]))
                });
            }
            _ => anyhow::bail!("pickup projection reply differs"),
        }
        self.pending = None;
        Ok(())
    }
    pub(super) fn advance(&mut self, s: &mut Session, state: &mut State) -> Result<Progress> {
        if let Some(r) = self.pending {
            return Ok(Progress::Service(Service::Pickup(r)));
        }
        match self.phase {
            Phase::Entry => {
                let w = &s.simulation.vm.world;
                state.preview_fields.as_mut().unwrap().value58 = w
                    .objects
                    .get(&self.input.picked)
                    .and_then(|o| o.multitile.as_ref())
                    .map_or(0, |p| p.offset[2]);
                let Some(p) = w.positions.get(&self.input.picked) else {
                    self.phase = Phase::Tail;
                    return Ok(Progress::Pending);
                };
                self.level = p.level;
                self.definition = state
                    .catalog()?
                    .entries
                    .iter()
                    .position(|e| e.name == b"Tile Cursor")
                    .map(|row| {
                        state
                            .history
                            .definitions
                            .get(&(row as u32 + 1))
                            .cloned()
                            .context("pickup cursor definition absent")
                    })
                    .transpose()?;
                if let Some(d) = &self.definition {
                    sims_content::assets::load_object_guids(&s.index, &mut s.program, &[d.guid])?;
                    s.simulation
                        .architecture
                        .as_mut()
                        .context("pickup architecture absent")?
                        .bind_footprints(&s.program)
                        .map_err(anyhow::Error::msg)?;
                }
                let clock = &s
                    .simulation
                    .household
                    .as_ref()
                    .context("pickup calendar absent")?
                    .clock;
                let date = sims_vm::construction::ObjectDate {
                    year: clock.year,
                    month: clock.month,
                    day: clock.day,
                };
                if s.simulation
                    .begin_cursor_factory(
                        &s.program,
                        self.definition.clone(),
                        self.input.picked,
                        self.level,
                        state.history.placement_cheat,
                        date,
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    self.phase = Phase::Factory;
                } else {
                    state.tool.held = None;
                    self.phase = Phase::Tail;
                }
            }
            Phase::Factory => {
                if s.house_notification_pending() {
                    return Ok(
                        match s.advance_house_notification(s.retained_camera_input()?, 1)? {
                            super::super::notifications::Progress::Service(r) => {
                                Progress::Service(Service::Notification(r))
                            }
                            _ => Progress::Pending,
                        },
                    );
                }
                let f = s.simulation.cursor_factory.as_ref().unwrap();
                if f.pending_focus().is_some()
                    || f.pending_created().is_some()
                    || f.returned().is_some()
                {
                    if let Some(object) =
                        s.service_cursor_factory_return(s.retained_camera_input()?, 1)?
                    {
                        state.tool.held = Some(
                            s.simulation
                                .vm
                                .world
                                .scheduler_storage
                                .as_ref()
                                .unwrap()
                                .handle(object)
                                .context("pickup returned cursor lifetime absent")?,
                        );
                        self.phase = Phase::Project;
                    }
                } else {
                    if f.placement().is_some() {
                        s.simulation
                            .begin_cursor_factory_placement(
                                &s.program,
                                state.history.placement_cheat,
                            )
                            .map_err(anyhow::Error::msg)?;
                    }
                    s.simulation
                        .advance_cursor_factory(&s.program, 1)
                        .map_err(anyhow::Error::msg)?;
                    s.move_tool_assets()?;
                }
            }
            Phase::Project => {
                let held = id(&s.simulation.vm.world, state.tool.held.unwrap())
                    .map_err(anyhow::Error::msg)?;
                let p = &s.simulation.vm.world.positions[&held];
                self.pending = Some(Request::Project {
                    position: [p.x, p.y],
                    level: current_floor(s)?,
                });
                self.phase = Phase::Classify;
            }
            Phase::Classify => {
                let held = id(&s.simulation.vm.world, state.tool.held.unwrap())
                    .map_err(anyhow::Error::msg)?;
                let h = s.simulation.vm.world.objects[&held]
                    .cursor
                    .as_ref()
                    .unwrap()
                    .underlying
                    .context("pickup cursor has no furniture")?;
                let target = id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
                s.simulation
                    .vm
                    .refresh_object(target, target, 0, 0)
                    .map_err(anyhow::Error::msg)?;
                let d = s.simulation.vm.world.objects[&target]
                    .definition
                    .as_ref()
                    .context("pickup definition absent")?;
                let handle = state
                    .history
                    .definition_handle(d)
                    .map_err(anyhow::Error::msg)?;
                state
                    .groups
                    .as_ref()
                    .context("pickup groups absent")?
                    .select_definition(
                        &mut state.tool.tool,
                        &mut s
                            .host
                            .presentation
                            .tool_selection
                            .as_mut()
                            .context("pickup selector absent")?
                            .masters,
                        state
                            .history
                            .catalog
                            .as_ref()
                            .context("pickup catalog absent")?,
                        handle,
                    )
                    .map_err(anyhow::Error::msg)?;
                //487ae6 writes the same+24 category later consumed by4929fb.
                state
                    .history
                    .budgets
                    .0
                    .get_mut(&self.tool)
                    .context("pickup budget owner absent")?
                    .category = state.tool.tool.cursor;
                ensure!(
                    state.tool.tool.mode == 1,
                    "pickup requires a special tool mode"
                );
                self.phase = Phase::Capture;
            }
            Phase::Capture => {
                preview::capture_command(s, state, self.tool, self.input.picked, 1, 0)?;
                state.tool.value90 = 0;
                state.history.budgets.0.get_mut(&self.tool).unwrap().pending = 0;
                self.phase = Phase::Tail;
            }
            Phase::Tail => {
                state.call.as_mut().unwrap().dispatch =
                    dispatch::Dispatch::cleanup(cleanup::Kind::Cursor);
                self.phase = Phase::Complete;
            }
            Phase::Complete => return Ok(Progress::Complete),
        }
        Ok(Progress::Pending)
    }
}
fn current_floor(s: &Session) -> Result<i32> {
    let n = s
        .host
        .presentation
        .presenter
        .as_ref()
        .context("pickup view absent")?
        .view
        .floor
        .level;
    Ok(if n == 0 { 1 } else { n })
}
///43219a's meaningful low word. Unbroken native returns leave the upper EAX
///bits unspecified; this canonicalizes them to zero. Broken prices use signed
///division by two. The selected-furniture and sale callers sign-extend AX.
pub(super) fn resale(s: &Session, target: ObjectId) -> Result<i32> {
    let w = &s.simulation.vm.world;
    let o = w.objects.get(&target).context("resale object absent")?;
    let words = sims_vm::construction::definition_words(
        &s.program,
        o.definition.as_ref().context("resale definition absent")?,
    )
    .map_err(anyhow::Error::msg)?;
    let parts = if words[10] != 0 {
        sims_vm::multitile::chain(w, target).map_err(anyhow::Error::msg)?
    } else {
        vec![target]
    };
    let first = *parts.first().context("resale multipart chain absent")?;
    let mut broken = false;
    for id in parts {
        broken |= *w.objects[&id]
            .data
            .get(&15)
            .context("resale broken word absent")?
            != 0;
    }
    let value = *w.objects[&first]
        .data
        .get(&41)
        .context("resale price absent")?;
    Ok(if broken {
        i32::from(value) / 2
    } else {
        i32::from(value as u16)
    })
}
impl Session {
    pub fn begin_move_tool_pickup(&mut self, input: motion::Input) -> Result<()> {
        self.check_move_tool_entry()?;
        let tool = self
            .host
            .presentation
            .current_tool
            .as_ref()
            .and_then(|c| c.current())
            .context("pickup current tool absent")?;
        ensure!(
            matches!(
                self.host
                    .presentation
                    .tool_selection
                    .as_ref()
                    .and_then(|t| t.receivers().get(&tool)),
                Some(super::super::tool_selection::Receiver::Move)
            ),
            "pickup current tool is not Move"
        );
        let state = self
            .host
            .presentation
            .move_tool
            .as_mut()
            .context("pickup tool absent")?;
        ensure!(
            state.tool.held.is_none() && state.tool.payload.is_none(),
            "pickup overlaps held object"
        );
        preview::bind_fields(state, tool)?;
        state.call = Some(super::Execution {
            button_owner: None,
            owner: None,
            current: None,
            tick: self.simulation.vm.tick,
            dispatch: dispatch::Dispatch::returned(),
            child: None,
            retired_held: None,
            retired_payload: None,
            preview: None,
            motion: None,
            commit: None,
            sale: None,
            pickup: Some(Execution::new(tool, input)),
        });
        Ok(())
    }
    pub fn acknowledge_move_pickup(&mut self, r: Request, reply: Reply) -> Result<()> {
        let state = self
            .host
            .presentation
            .move_tool
            .as_mut()
            .context("pickup tool absent")?;
        let mut p = state
            .call
            .as_mut()
            .and_then(|c| c.pickup.take())
            .context("pickup caller absent")?;
        let result = p.acknowledge(state, r, reply);
        state.call.as_mut().unwrap().pickup = Some(p);
        result
    }
}
