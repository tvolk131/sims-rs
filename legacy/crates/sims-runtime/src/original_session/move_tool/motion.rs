//! Ordinary held-furniture branch of485a2e. The frontend supplies a point on
//! its rendered floor; original detach/undo/acceptance/place/rotation ordering
//! remains here. Includes drag orientation and the no-held hover tail;
//! door/window selling still requires its distinct caller.
use super::*;
use sims_sim::lot_cursor::rotation::{Point, PointResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    ///Physical lot Y/X in sixteenths of a tile, before485f6d snapping.
    pub position: [i32; 2],
    ///Pointer vertical/horizontal coordinates, narrowed like the event words.
    pub screen: [i16; 2],
    ///Original picked script ID, supplied by the renderer for the no-held tail.
    #[serde(default, skip_serializing_if = "no_pick")]
    pub picked: ObjectId,
}
fn no_pick(id: &ObjectId) -> bool {
    *id == 0
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Start,
    Enabled,
    Stop,
    Restored,
    DragOriented,
    DragChecked,
    Detached,
    Hovered,
    Checked,
    Placed,
    Rotated,
    Unrotated,
    FallbackChecked,
    Publish,
    Tail,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Execution {
    input: Input,
    held: u32,
    phase: Phase,
    old: [i32; 2],
    level: i32,
    slot: i32,
    point: Option<Point>,
    rotation: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    orientation: Option<i32>,
    checked: Option<PointResult>,
    attempt: i32,
    attempts: i32,
}
fn cursor<'a>(s: &'a Session, held: u32) -> Result<&'a sims_vm::cursor::State> {
    s.simulation.vm.world.objects[&id(&s.simulation.vm.world, held).map_err(anyhow::Error::msg)?]
        .cursor
        .as_ref()
        .context("pointer cursor absent")
}
pub(super) fn capture(s: &mut Session, h: u32) -> Result<sims_sim::lot_links::Descriptor> {
    let object = id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
    s.simulation
        .native_lot
        .as_ref()
        .context("pointer physical lot absent")?
        .placement
        .links
        .capture(&mut s.simulation.vm.world, object)
        .map_err(anyhow::Error::msg)
}
fn valid(s: &Session, h: u32) -> Result<bool> {
    let c = cursor(s, h)?;
    Ok(c.underlying.is_some() && c.valid != 0)
}
///43583e: wall-placement bits, or a multipart group's mixed foundation flags.
fn can_auto_rotate(s: &Session, h: u32) -> Result<bool> {
    let Some(h) = cursor(s, h)?.underlying else {
        return Ok(false);
    };
    let w = &s.simulation.vm.world;
    let object = id(w, h).map_err(anyhow::Error::msg)?;
    let word = |id: ObjectId, n| {
        w.objects[&id]
            .data
            .get(&n)
            .copied()
            .context("auto-rotation word absent")
    };
    if word(object, 13)? & 15 != 0 {
        return Ok(true);
    }
    let d = w.objects[&object]
        .definition
        .as_ref()
        .context("auto-rotation definition absent")?;
    if sims_vm::construction::definition_words(&s.program, d).map_err(anyhow::Error::msg)?[10] == 0
    {
        return Ok(false);
    }
    let chain = sims_vm::multitile::chain(w, object).map_err(anyhow::Error::msg)?;
    let mut foundation = false;
    let mut ordinary = false;
    for id in chain {
        if word(id, 13)? & 15 != 0 {
            return Ok(true);
        }
        if word(id, 42)? & 0x800 != 0 {
            foundation = true;
        } else {
            ordinary = true;
        }
    }
    Ok(foundation && ordinary)
}
pub(super) fn snap(p: [i32; 2]) -> [i32; 2] {
    p.map(|v| (v & !7) | 8)
}
impl Execution {
    pub(super) fn new(input: Input, held: u32) -> Self {
        Self {
            input,
            held,
            phase: Phase::Start,
            old: [0; 2],
            level: 0,
            slot: 0,
            point: None,
            rotation: None,
            orientation: None,
            checked: None,
            attempt: 0,
            attempts: 0,
        }
    }
    pub(super) fn complete(&self) -> bool {
        self.phase == Phase::Complete && !self.owns_cursor()
    }
    pub(super) fn owns_cursor(&self) -> bool {
        self.point.is_some() || self.rotation.is_some() || self.orientation.is_some()
    }
    pub(super) fn validate(&self, s: &Session, state: &State) -> Result<()> {
        ensure!(
            state.tool.held == (self.held != 0).then_some(self.held)
                && (self.held == 0 || state.preview_fields.is_some()),
            "pointer lost held preview"
        );
        ensure!(
            matches!(state.tool.tool.mode, 1 | 2),
            "pointer branch changed during movement"
        );
        ensure!(
            [
                self.point.is_some(),
                self.rotation.is_some(),
                self.orientation.is_some()
            ]
            .into_iter()
            .filter(|b| *b)
            .count()
                <= 1,
            "pointer owns two cursor calls"
        );
        let root = s.simulation.cursor_rotation.as_ref();
        if let Some(point) = self.point {
            ensure!(
                root.is_some_and(|r| r.point() == Some(point) && r.binding().0 == self.held),
                "pointer lost its exact point call"
            );
        }
        if let Some(n) = self.rotation {
            ensure!(
                root.is_some_and(|r| r.point().is_none()
                    && r.orientation().is_none()
                    && !r.removal()
                    && r.binding() == (self.held, n)),
                "pointer lost its exact rotation call"
            );
        }
        if let Some(n) = self.orientation {
            ensure!(
                root.is_some_and(|r| r.orientation() == Some(n) && r.binding().0 == self.held),
                "pointer lost its orientation child"
            );
        }
        if self.owns_cursor() {
            ensure!(
                state.call.as_ref().unwrap().dispatch.complete(),
                "pointer cursor overlaps Move child"
            );
        }
        Ok(())
    }
    fn begin_point(
        &mut self,
        s: &mut Session,
        state: &State,
        point: Point,
        after: Phase,
    ) -> Result<()> {
        s.simulation
            .begin_cursor_point(
                &s.program,
                id(&s.simulation.vm.world, self.held).map_err(anyhow::Error::msg)?,
                point,
                state.history.placement_cheat,
            )
            .map_err(anyhow::Error::msg)?;
        self.point = Some(point);
        self.checked = None;
        self.phase = after;
        Ok(())
    }
    fn check(
        &mut self,
        s: &mut Session,
        state: &State,
        position: [i32; 2],
        after: Phase,
    ) -> Result<()> {
        self.begin_point(
            s,
            state,
            Point::Check {
                position,
                level: self.level,
            },
            after,
        )
    }
    fn place(
        &mut self,
        s: &mut Session,
        state: &State,
        position: [i32; 2],
        slot: i32,
        after: Phase,
    ) -> Result<()> {
        self.begin_point(
            s,
            state,
            Point::Place {
                position,
                level: self.level,
                slot,
                notify: true,
            },
            after,
        )
    }
    fn accepted(&mut self) -> Result<(bool, [i32; 2])> {
        let Some(PointResult::Checked { outcome, position }) = self.checked.take() else {
            anyhow::bail!("pointer acceptance result absent");
        };
        Ok((outcome.accepted, position))
    }
    fn rotate(&mut self, s: &mut Session, state: &State, n: i32, after: Phase) -> Result<()> {
        s.simulation
            .begin_cursor_rotation(
                &s.program,
                id(&s.simulation.vm.world, self.held).map_err(anyhow::Error::msg)?,
                n,
                state.history.placement_cheat,
            )
            .map_err(anyhow::Error::msg)?;
        self.rotation = Some(n);
        self.phase = after;
        Ok(())
    }
    pub(super) fn advance(&mut self, s: &mut Session, state: &mut State) -> Result<Progress> {
        ensure!(
            state.call.as_ref().unwrap().dispatch.complete()
                && state.call.as_ref().unwrap().child.is_none(),
            "pointer child has not returned"
        );
        if self.owns_cursor() {
            if self.point.is_some() {
                s.simulation.advance_cursor_point(&s.program, 1)
            } else if self.orientation.is_some() {
                s.simulation.advance_cursor_orientation(&s.program, 1)
            } else {
                s.simulation.advance_cursor_rotation(&s.program, 1)
            }
            .map_err(anyhow::Error::msg)?;
            s.move_tool_assets()?;
            if s.simulation.cursor_rotation.as_ref().unwrap().complete() {
                if self.point.take().is_some() {
                    self.checked = Some(
                        s.simulation
                            .finish_cursor_point(&s.program)
                            .map_err(anyhow::Error::msg)?,
                    );
                } else if self.orientation.take().is_some() {
                    s.simulation
                        .finish_cursor_orientation(&s.program)
                        .map_err(anyhow::Error::msg)?;
                } else {
                    s.simulation
                        .finish_cursor_rotation(&s.program)
                        .map_err(anyhow::Error::msg)?;
                    self.rotation = None;
                }
            }
            return Ok(Progress::Pending);
        }
        match self.phase {
            Phase::Start => {
                //Control has no special meaning in ordinary furniture mode.
                state.tool.set_byte(0x79, 0);
                state.tool.set_byte(0x7a, 0);
                if self.held == 0 {
                    let picked = s
                        .simulation
                        .vm
                        .world
                        .objects
                        .contains_key(&self.input.picked)
                        .then_some(self.input.picked)
                        .unwrap_or(0);
                    if picked == 0 {
                        state.tool.set_byte(0x80, 0);
                    }
                    state.call.as_mut().unwrap().dispatch = dispatch::Dispatch::hover(picked, 0);
                    self.phase = Phase::Tail;
                    return Ok(Progress::Pending);
                }
                let h = cursor(s, self.held)?
                    .underlying
                    .context("pointer furniture absent")?;
                let descriptor = capture(s, h)?;
                self.slot = if descriptor.container.is_some() {
                    capture(s, h)?.slot as i16 as i32
                } else {
                    0
                };
                let object = id(&s.simulation.vm.world, self.held).map_err(anyhow::Error::msg)?;
                let position = s
                    .simulation
                    .vm
                    .world
                    .positions
                    .get(&object)
                    .context("pointer physical position absent")?;
                self.old = [position.x, position.y];
                if capture(s, self.held)?.on_lot {
                    state.call.as_mut().unwrap().dispatch = dispatch::Dispatch::detach(self.held);
                }
                self.phase = Phase::Enabled;
            }
            Phase::Enabled => {
                if state.tool.byte(0x97) == 0 {
                    self.phase = Phase::Complete;
                    return Ok(Progress::Complete);
                }
                self.level = s
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .context("pointer floor absent")?
                    .view
                    .floor
                    .level;
                if self.level == 0 {
                    self.level = 1;
                }
                state.last_pointer = Some(self.input.screen);
                if let Some(payload) = state.tool.payload {
                    state.call.as_mut().unwrap().dispatch =
                        dispatch::Dispatch::stop_payload(payload);
                }
                self.phase = Phase::Stop;
            }
            Phase::Stop => self.place(s, state, self.old, self.slot, Phase::Restored)?,
            Phase::Restored => {
                if state.tool.byte(0x7b) != 0 {
                    let delta = std::array::from_fn::<_, 2, _>(|i| {
                        i32::from(self.input.screen[i]).wrapping_sub(state.button_fields.down[i])
                    });
                    let distance = delta[0]
                        .wrapping_mul(delta[0])
                        .wrapping_add(delta[1].wrapping_mul(delta[1]));
                    if distance > 36 {
                        state.tool.set_byte(0x80, 0);
                        state.tool.result = match (delta[0] < 0, delta[1] < 0) {
                            (true, true) => 6,
                            (true, false) => 4,
                            (false, true) => 0,
                            (false, false) => 2,
                        };
                        let camera = s
                            .host
                            .presentation
                            .presenter
                            .as_ref()
                            .context("drag camera absent")?
                            .view
                            .display_rotation as i32;
                        let desired = state.tool.result.wrapping_sub(camera.wrapping_mul(2)) & 7;
                        let h = cursor(s, self.held)?
                            .underlying
                            .context("drag furniture absent")?;
                        let object = id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
                        let facing = i32::from(
                            *s.simulation.vm.world.objects[&object]
                                .data
                                .get(&1)
                                .context("drag facing absent")?,
                        );
                        if facing != desired {
                            let definition = s.simulation.vm.world.objects[&object]
                                .definition
                                .as_ref()
                                .context("drag definition absent")?;
                            let handle = state
                                .history
                                .definition_handle(definition)
                                .map_err(anyhow::Error::msg)?;
                            let catalog = state.catalog()?;
                            let master = s
                                .host
                                .presentation
                                .tool_selection
                                .as_mut()
                                .context("drag master cache absent")?
                                .masters
                                .resolve(catalog, handle)
                                .map_err(anyhow::Error::msg)?;
                            let offset = master
                                .map(|m| {
                                    i16::from_le_bytes(
                                        catalog.entries[m as usize - 1].data[0x7a..0x7c]
                                            .try_into()
                                            .unwrap(),
                                    )
                                })
                                .unwrap_or(0);
                            let orientation = desired.wrapping_sub(i32::from(offset)) & 7;
                            s.simulation
                                .begin_cursor_orientation(
                                    &s.program,
                                    id(&s.simulation.vm.world, self.held)
                                        .map_err(anyhow::Error::msg)?,
                                    orientation,
                                    state.history.placement_cheat,
                                )
                                .map_err(anyhow::Error::msg)?;
                            self.orientation = Some(orientation);
                            self.phase = Phase::DragOriented;
                        } else {
                            self.check(s, state, self.old, Phase::DragChecked)?;
                        }
                        return Ok(Progress::Pending);
                    }
                    state.tool.result = -1;
                }
                state.tool.set_byte(0x95, 0);
                if capture(s, self.held)?.on_lot {
                    state.call.as_mut().unwrap().dispatch = dispatch::Dispatch::detach(self.held);
                }
                self.phase = Phase::Detached;
            }
            Phase::DragOriented => {
                state.preview_fields.as_mut().unwrap().points[2..].fill(-16);
                state.tool.set_byte(0x96, 1);
                self.check(s, state, self.old, Phase::DragChecked)?;
            }
            Phase::DragChecked => {
                let (accepted, point) = self.accepted()?;
                if accepted {
                    self.place(s, state, point, self.slot, Phase::Publish)?;
                } else {
                    self.phase = Phase::Publish;
                }
            }
            Phase::Detached => {
                if let Some(h) = cursor(s, self.held)?.underlying {
                    state.call.as_mut().unwrap().dispatch = dispatch::Dispatch::hover(
                        id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?,
                        0,
                    );
                }
                self.phase = Phase::Hovered;
            }
            Phase::Hovered => {
                let point = snap(self.input.position);
                //422632 narrows shifted coordinates to signed bytes.
                let size = s
                    .simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("pointer lot absent")?
                    .size as i32;
                if point.iter().any(|v| {
                    let n = (v >> 4) as i8 as i32;
                    n <= 0 || n >= size - 1
                }) {
                    self.phase = Phase::Tail;
                } else {
                    self.check(s, state, point, Phase::Checked)?;
                }
            }
            Phase::Checked => {
                let (accepted, point) = self.accepted()?;
                if accepted {
                    self.place(s, state, point, self.slot, Phase::Placed)?;
                } else {
                    self.check(s, state, self.input.position, Phase::FallbackChecked)?;
                }
            }
            Phase::Placed => {
                if valid(s, self.held)? {
                    self.phase = Phase::Publish;
                } else if can_auto_rotate(s, self.held)? {
                    let object =
                        id(&s.simulation.vm.world, self.held).map_err(anyhow::Error::msg)?;
                    let step = *s.simulation.vm.world.objects[&object]
                        .data
                        .get(&23)
                        .context("rotation step absent")?;
                    ensure!(step != 0, "native automatic rotation divides by zero");
                    self.attempts = 8 / i32::from(step);
                    self.attempt = 1;
                    self.rotate(s, state, 1, Phase::Rotated)?;
                } else {
                    self.check(s, state, self.input.position, Phase::FallbackChecked)?;
                }
            }
            Phase::Rotated => {
                if valid(s, self.held)? {
                    self.phase = Phase::Publish;
                } else {
                    self.rotate(s, state, -self.attempt, Phase::Unrotated)?;
                }
            }
            Phase::Unrotated => {
                self.attempt += 1;
                if self.attempt < self.attempts {
                    self.rotate(s, state, self.attempt, Phase::Rotated)?;
                } else {
                    self.check(s, state, self.input.position, Phase::FallbackChecked)?;
                }
            }
            Phase::FallbackChecked => {
                let (accepted, point) = self.accepted()?;
                if accepted {
                    self.place(s, state, point, 0, Phase::Publish)?;
                } else {
                    state.call.as_mut().unwrap().dispatch = dispatch::Dispatch::detach(self.held);
                    self.phase = Phase::Publish;
                }
            }
            Phase::Publish => {
                //486df7: publish validity, clear the cursor's graphic word for
                //the whole group, then invalidate each part through434101.
                let accepted = valid(s, self.held)?;
                state.tool.set_byte(0x95, u8::from(accepted));
                let object = id(&s.simulation.vm.world, self.held).map_err(anyhow::Error::msg)?;
                if *s.simulation.vm.world.objects[&object]
                    .data
                    .get(&0)
                    .context("cursor graphic word absent")?
                    != 0
                {
                    let parts = sims_vm::multitile::chain(&s.simulation.vm.world, object)
                        .map_err(anyhow::Error::msg)?;
                    for &id in &parts {
                        s.simulation
                            .vm
                            .world
                            .objects
                            .get_mut(&id)
                            .unwrap()
                            .data
                            .insert(0, 0);
                    }
                    for id in parts {
                        s.simulation
                            .vm
                            .refresh_object(id, id, 0, 0)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                let fields = state.preview_fields.as_mut().unwrap();
                if accepted {
                    let p = &s.simulation.vm.world.positions[&object];
                    fields.points[..2].copy_from_slice(&[p.x, p.y]);
                }
                state.tool.set_byte(0x96, state.tool.byte(0x95));
                fields.points[2] = fields.points[0];
                fields.points[3] = fields.points[1];
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
