//!486f5a's concrete held-object update. Creation, rotation, history capture
//!and hover are real engine children; this routine never commits a purchase.
use super::*;
use sims_vm::{Definition, cursor::orientation::camera_facing};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub(super) direction: i32,        //+40
    previous_definition: Option<u32>, //+48
    pub(super) value58: i32,
    pub(super) points: [i32; 4], //+9c..a8
    pub(super) budget: u32,
    next_command: u32,
}
impl Fields {
    pub(super) fn validate(&self, state: &State) -> Result<()> {
        ensure!(
            self.budget != 0
                && self.next_command != 0
                && self.previous_definition != Some(0)
                && state.history.budgets.0.contains_key(&self.budget),
            "preview fields lost their owner"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Start,
    Pointer,
    Definition,
    Purchase,
    Orient,
    History,
    Offsets,
    Hover,
    Tail,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Execution {
    tool: u32,
    phase: Phase,
    definition: Option<Definition>,
    level: i32,
    underlying: Option<u32>,
    ///487d3d owns the +97 store after the complete virtual+14 return.
    ///476a35 calls +14 directly and must not acquire this tail.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    viewport_entry: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    viewport_returned: bool,
}
impl Execution {
    pub(super) fn new(tool: u32) -> Self {
        Self {
            tool,
            phase: Phase::Start,
            definition: None,
            level: 0,
            underlying: None,
            viewport_entry: false,
            viewport_returned: false,
        }
    }
    pub(super) fn viewport_entry(tool: u32) -> Self {
        Self {
            viewport_entry: true,
            ..Self::new(tool)
        }
    }
    pub(super) fn complete(&self) -> bool {
        self.phase == Phase::Complete && (!self.viewport_entry || self.viewport_returned)
    }
    pub(super) fn creating(&self) -> bool {
        self.phase == Phase::Purchase
    }
    pub(super) fn orienting(&self) -> bool {
        self.phase == Phase::Orient
    }
    pub(super) fn input(&self, state: &State) -> Option<cleanup::Request> {
        (self.phase == Phase::Pointer && state.call.as_ref()?.dispatch.complete()).then_some(
            cleanup::Request::SetCursor {
                window: state.inputs.window,
                value: 0,
            },
        )
    }
    pub(super) fn acknowledge(
        &mut self,
        request: cleanup::Request,
        reply: cleanup::Reply,
        window: u32,
    ) -> Result<()> {
        ensure!(
            self.phase == Phase::Pointer
                && request == cleanup::Request::SetCursor { window, value: 0 }
                && reply == cleanup::Reply::Done,
            "preview cursor reply differs"
        );
        self.phase = Phase::Definition;
        Ok(())
    }
    pub(super) fn validate(&self, s: &Session, state: &State) -> Result<()> {
        ensure!(self.tool != 0, "preview tool absent");
        ensure!(
            !self.viewport_returned || self.viewport_entry && self.phase == Phase::Complete,
            "viewport entry tail lost its preview return"
        );
        if let Some(fields) = &state.preview_fields {
            ensure!(fields.budget == self.tool, "preview budget owner changed");
        }
        ensure!(
            self.creating()
                == s.host
                    .presentation
                    .purchase_cursor
                    .as_ref()
                    .is_some_and(|p| p.move_owned()),
            "preview purchase owner differs"
        );
        if self.creating() {
            ensure!(
                s.host
                    .presentation
                    .purchase_cursor
                    .as_ref()
                    .unwrap()
                    .matches_move(self.definition.as_ref(), self.level),
                "preview purchase arguments changed"
            );
        }
        if self.orienting() {
            let fields = state
                .preview_fields
                .as_ref()
                .context("preview direction absent")?;
            ensure!(
                s.simulation
                    .cursor_rotation
                    .as_ref()
                    .is_some_and(|r| r.orientation() == Some(fields.direction)
                        && Some(r.binding().0) == state.tool.held),
                "preview orientation lost its child"
            );
        }
        if let Some(h) = self.underlying {
            id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub(super) fn advance(&mut self, s: &mut Session, state: &mut State) -> Result<Progress> {
        ensure!(
            state.call.as_ref().unwrap().dispatch.complete()
                && state.call.as_ref().unwrap().child.is_none(),
            "preview advanced before its Move child returned"
        );
        match self.phase {
            Phase::Start => {
                if state.tool.tool.flags[0] == 0 {
                    state.call.as_mut().unwrap().dispatch =
                        dispatch::Dispatch::cleanup(cleanup::Kind::Cursor);
                    self.phase = Phase::Complete;
                } else {
                    bind_fields(state, self.tool)?;
                    state.call.as_mut().unwrap().dispatch =
                        dispatch::Dispatch::cleanup(cleanup::Kind::Reset);
                    self.phase = Phase::Pointer;
                }
            }
            Phase::Pointer => {
                return Ok(Progress::Service(Service::Input(
                    self.input(state).context("preview cursor caller absent")?,
                )));
            }
            Phase::Definition => {
                let selected = state
                    .tool
                    .tool
                    .definition
                    .context("preview definition absent")?;
                let fields = state
                    .preview_fields
                    .as_mut()
                    .context("preview fields absent")?;
                if fields.previous_definition != Some(selected) {
                    let view = &s
                        .host
                        .presentation
                        .presenter
                        .as_ref()
                        .context("preview view absent")?
                        .view;
                    fields.direction = camera_facing(view.display_rotation as i32);
                }
                fields.previous_definition = Some(selected);
                let entry = state
                    .catalog()?
                    .entries
                    .get(selected as usize - 1)
                    .context("preview catalog definition absent")?;
                let price = i16::from_le_bytes(entry.data[0x24..0x26].try_into().unwrap());
                let funds = if *s
                    .simulation
                    .vm
                    .world
                    .globals
                    .get(&32)
                    .context("preview lot kind absent")?
                    == 1
                {
                    999999
                } else {
                    s.simulation
                        .vm
                        .world
                        .budget
                        .as_ref()
                        .context("preview funds absent")?
                        .balance
                };
                if i32::from(price) > funds {
                    self.phase = Phase::Tail;
                    return Ok(Progress::Pending);
                }
                self.level = if state.tool.tool.mode == 5 {
                    1
                } else {
                    let n = s
                        .host
                        .presentation
                        .presenter
                        .as_ref()
                        .context("preview floor absent")?
                        .view
                        .floor
                        .level;
                    if n == 0 { 1 } else { n }
                };
                self.definition = Some(
                    state
                        .history
                        .definitions
                        .get(&selected)
                        .context("preview resource absent")?
                        .clone(),
                );
                let cursor = state
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
                            .context("cursor resource absent")
                    })
                    .transpose()?;
                s.begin_move_purchase_cursor(
                    self.definition.clone(),
                    cursor,
                    self.level,
                    state.history.placement_cheat,
                )?;
                self.phase = Phase::Purchase;
            }
            Phase::Purchase => {
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
                if s.advance_purchase_cursor(1)? {
                    let result = s.finish_purchase_cursor()?;
                    state.tool.held = result
                        .map(|o| {
                            s.simulation
                                .vm
                                .world
                                .scheduler_storage
                                .as_ref()
                                .and_then(|r| r.handle(o))
                                .context("preview returned cursor lifetime absent")
                        })
                        .transpose()?;
                    let Some(cursor) = result else {
                        self.phase = Phase::Complete;
                        return Ok(Progress::Pending);
                    };
                    let fields = state.preview_fields.as_mut().unwrap();
                    fields.value58 = 0;
                    self.underlying = s.simulation.vm.world.objects[&cursor]
                        .cursor
                        .as_ref()
                        .context("preview result is not cursor")?
                        .underlying;
                    s.simulation
                        .begin_cursor_orientation(
                            &s.program,
                            cursor,
                            fields.direction,
                            state.history.placement_cheat,
                        )
                        .map_err(anyhow::Error::msg)?;
                    self.phase = Phase::Orient;
                }
            }
            Phase::Orient => {
                s.simulation
                    .advance_cursor_orientation(&s.program, 1)
                    .map_err(anyhow::Error::msg)?;
                s.move_tool_assets()?;
                if s.simulation.cursor_rotation.as_ref().unwrap().complete() {
                    s.simulation
                        .finish_cursor_orientation(&s.program)
                        .map_err(anyhow::Error::msg)?;
                    self.phase = if self.underlying.is_some() {
                        Phase::History
                    } else {
                        Phase::Tail
                    };
                }
            }
            Phase::History => {
                let cost = price(s, state)?;
                let script_id = id(
                    &s.simulation.vm.world,
                    self.underlying
                        .context("preview captured furniture absent")?,
                )
                .map_err(anyhow::Error::msg)?;
                capture_command(s, state, self.tool, script_id, 0, cost)?;
                state.tool.value90 = cost as u32;
                self.phase = Phase::Offsets;
            }
            Phase::Offsets => {
                match state.tool.tool.mode {
                    3 | 4 => {
                        anyhow::bail!("preview requires original door/window pointer offsets487241")
                    }
                    5 => state.tool.tool.position = [0; 2],
                    _ => (),
                }
                let target = id(
                    &s.simulation.vm.world,
                    self.underlying.context("preview hover furniture absent")?,
                )
                .map_err(anyhow::Error::msg)?;
                state.call.as_mut().unwrap().dispatch = dispatch::Dispatch::hover(target, 0);
                self.phase = Phase::Hover;
            }
            Phase::Hover => {
                for offset in [0x97, 0x98, 0x95, 0x96] {
                    state.tool.set_byte(offset, 0);
                }
                state.preview_fields.as_mut().unwrap().points = [-16; 4];
                self.phase = Phase::Tail;
            }
            Phase::Tail => {
                state.call.as_mut().unwrap().dispatch =
                    dispatch::Dispatch::cleanup(cleanup::Kind::Cursor);
                self.phase = Phase::Complete;
            }
            Phase::Complete => {
                if self.viewport_entry {
                    state.tool.set_byte(0x97, 1);
                    self.viewport_returned = true;
                }
                return Ok(Progress::Complete);
            }
        }
        Ok(Progress::Pending)
    }
}
pub(super) fn bind_fields(state: &mut State, tool: u32) -> Result<()> {
    if state.preview_fields.is_none() {
        //First binding of this replacement tool's previously
        //unused preview fields. Explicit485780/49290f stores.
        ensure!(
            state.tool.held.is_none() && state.tool.payload.is_none(),
            "historical held tool needs its retained preview fields"
        );
        state
            .history
            .budgets
            .0
            .entry(tool)
            .or_insert(object::budget::State {
                pending: 0,
                category: state.tool.tool.cursor,
            });
        let next_command = state
            .history
            .store
            .commands
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .context("history identities exhausted")?;
        state.preview_fields = Some(Fields {
            direction: 0,
            previous_definition: None,
            value58: 0,
            points: [0; 4],
            budget: tool,
            next_command,
        });
    }
    Ok(())
}
pub(super) fn capture_command(
    s: &mut Session,
    state: &mut State,
    tool: u32,
    script_id: ObjectId,
    kind: i32,
    cost: i32,
) -> Result<()> {
    let fields = state.preview_fields.as_mut().unwrap();
    let mut allocate = || {
        let id = fields.next_command;
        fields.next_command = id
            .checked_add(1)
            .context("history identities exhausted")
            .map_err(|e| e.to_string())?;
        Ok(Some(id))
    };
    let command = allocate().map_err(anyhow::Error::msg)?.unwrap();
    let definitions = &state.history.definitions;
    let definition = |d: &Definition| {
        let mut matches = definitions
            .iter()
            .filter_map(|(&id, v)| (v == d).then_some(id));
        let id = matches.next().ok_or("snapshot definition absent")?;
        if matches.next().is_some() {
            return Err("ambiguous snapshot definition".into());
        }
        Ok(id)
    };
    let mut view = snapshot::live::View {
        world: &mut s.simulation.vm.world,
        program: &s.program,
        links: &s
            .simulation
            .native_lot
            .as_ref()
            .context("preview lot links absent")?
            .placement
            .links,
        definition: &definition,
    };
    object::construction::construct(
        object::construction::Input {
            command,
            script_id,
            kind,
            cost,
            budget: tool,
        },
        &mut state.history.store,
        &mut state.history.objects,
        &mut state.history.references,
        &mut view,
        &mut allocate,
    )
    .map_err(anyhow::Error::msg)?;
    state.tool.payload = Some(command);
    state
        .history
        .store
        .acquire(command)
        .map_err(anyhow::Error::msg)?;
    Ok(())
}
pub(super) fn price(s: &mut Session, state: &State) -> Result<i32> {
    let cursor = id(
        &s.simulation.vm.world,
        state.tool.held.context("preview held cursor absent")?,
    )
    .map_err(anyhow::Error::msg)?;
    let Some(h) = s.simulation.vm.world.objects[&cursor]
        .cursor
        .as_ref()
        .context("preview cursor absent")?
        .underlying
    else {
        return Ok(0);
    };
    let target = id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
    let definition = s.simulation.vm.world.objects[&target]
        .definition
        .as_ref()
        .context("preview furniture definition absent")?;
    let mut handle = state
        .history
        .definition_handle(definition)
        .map_err(anyhow::Error::msg)?;
    let catalog = state.catalog()?;
    if i16::from_le_bytes(
        catalog.entries[handle as usize - 1].data[0x14..0x16]
            .try_into()
            .unwrap(),
    ) != 0
    {
        handle = s
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("preview master cache absent")?
            .masters
            .resolve(catalog, handle)
            .map_err(anyhow::Error::msg)?
            .context("preview multipart master absent")?;
    }
    Ok(i32::from(i16::from_le_bytes(
        catalog.entries[handle as usize - 1].data[0x24..0x26]
            .try_into()
            .unwrap(),
    )))
}
impl State {
    pub(in crate::original_session) fn preview_creating(&self) -> bool {
        self.call
            .as_ref()
            .and_then(|c| c.preview.as_ref())
            .is_some_and(Execution::creating)
    }
    pub(super) fn preview_orienting(&self) -> bool {
        self.call
            .as_ref()
            .and_then(|c| c.preview.as_ref())
            .is_some_and(Execution::orienting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_pointer_replies_require_exact_window_value_and_continuation() {
        use cleanup::{Reply, Request};
        let mut call = Execution::new(7000);
        let correct = Request::SetCursor {
            window: 37,
            value: 0,
        };
        let before = call.clone();
        assert!(call.acknowledge(correct, Reply::Done, 37).is_err());
        assert_eq!(call, before);
        call.phase = Phase::Pointer;
        let before = call.clone();
        for (request, reply) in [
            (
                Request::SetCursor {
                    window: 38,
                    value: 0,
                },
                Reply::Done,
            ),
            (
                Request::SetCursor {
                    window: 37,
                    value: 1,
                },
                Reply::Done,
            ),
            (correct, Reply::Key(0)),
        ] {
            assert!(call.acknowledge(request, reply, 37).is_err());
            assert_eq!(call, before);
        }
        let mut resumed: Execution =
            serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
        call.acknowledge(correct, Reply::Done, 37).unwrap();
        resumed.acknowledge(correct, Reply::Done, 37).unwrap();
        assert_eq!(call, resumed);
        assert_eq!(call.phase, Phase::Definition);
        let before = call.clone();
        assert!(call.acknowledge(correct, Reply::Done, 37).is_err());
        assert_eq!(call, before);
    }
}
