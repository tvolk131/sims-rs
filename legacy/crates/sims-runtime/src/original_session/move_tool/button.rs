//!Saved player button callers485f98/4864d0 for ordinary Buy furniture.
//!The concrete Move, tool-selector and DA children keep their own execution.
use super::*;
use crate::original_session::{notifications, tool_selection};
use sims_vm::budget::Application;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub down: [i32; 2], //+5c,+60
    pub last: [i32; 2], //+64,+68
    pub hint: Option<Hint>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub picked: ObjectId, //+84
}
fn is_zero(v: &ObjectId) -> bool {
    *v == 0
}
impl Fields {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hint {
    Placement(i32),
    Funds,
    Spent(i32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Down,
    Up,
    Cancel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub kind: Kind,
    pub pointer: motion::Input,
    pub shift: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Operation {
    Motion(motion::Input),
    Commit,
    Pickup(motion::Input),
    Reset,
    Cursor,
    Preview,
    Cancel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Down,
    Picked,
    PickupSelected,
    Up,
    Committed,
    Reset,
    Budget,
    Charged,
    Repeat,
    Repeated,
    Selected,
    SelectionCleared,
    Tail,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Child {
    Move(Operation),
    Selector,
    Budget(Application),
    ClearProducts,
    Selection(pickup::Request),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    tick: u64,
    input: Input,
    phase: Phase,
    child: Option<Child>,
    cost: i32,
    budget: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Service {
    Move(super::Service),
    Tool(tool_selection::Service),
    Notification(notifications::Service),
    ClearProducts,
    Selection(pickup::Request),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    Service(Service),
}
fn funds(s: &Session) -> Result<i32> {
    if *s
        .simulation
        .vm
        .world
        .globals
        .get(&32)
        .context("Buy lot kind absent")?
        == 1
    {
        Ok(999999)
    } else {
        Ok(s.simulation
            .vm
            .world
            .budget
            .as_ref()
            .context("Buy funds absent")?
            .balance)
    }
}
fn distance(a: [i32; 2], b: [i32; 2]) -> i32 {
    let x = a[0].wrapping_sub(b[0]);
    let y = a[1].wrapping_sub(b[1]);
    x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y))
}
impl State {
    pub(super) fn validate(&self, s: &Session) -> Result<()> {
        let binding = match self.child.as_ref() {
            Some(Child::Move(Operation::Motion(p))) => {
                *p == self.input.pointer
                    && matches!(self.phase, Phase::Down | Phase::Up | Phase::Complete)
            }
            Some(Child::Move(Operation::Pickup(p))) => {
                self.phase == Phase::Picked && *p == self.input.pointer
            }
            Some(Child::Selection(_)) => self.phase == Phase::PickupSelected,
            Some(Child::Move(Operation::Commit)) => self.phase == Phase::Committed,
            Some(Child::Move(Operation::Reset)) => self.phase == Phase::Reset,
            Some(Child::Move(Operation::Cursor)) => {
                self.phase == Phase::Complete && self.input.kind == Kind::Down
            }
            Some(Child::Move(Operation::Preview)) => self.phase == Phase::Repeated,
            Some(Child::Move(Operation::Cancel)) => {
                self.phase == Phase::Complete && self.input.kind == Kind::Cancel
            }
            Some(Child::Selector) => self.phase == Phase::Selected,
            Some(Child::Budget(_)) => self.phase == Phase::Charged && self.budget != 0,
            Some(Child::ClearProducts) => self.phase == Phase::SelectionCleared,
            None => true,
        };
        ensure!(binding, "button child lost its continuation");
        ensure!(
            self.tick == s.simulation.vm.tick
                && !s.clock_pending()
                && !s.transport_pending()
                && !s.ground_input_pending()
                && !s.object_input_pending(),
            "Move button overlaps gameplay work"
        );
        let m = s
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("button Move absent")?;
        match &self.child {
            Some(Child::Move(op)) => ensure!(
                m.call.as_ref().and_then(|c| c.button_owner) == Some(*op),
                "button lost exact Move child"
            ),
            Some(Child::Selector) => ensure!(
                s.host
                    .presentation
                    .tool_selection
                    .as_ref()
                    .is_some_and(|t| t.button_owned()),
                "button lost selector child"
            ),
            Some(Child::Budget(a)) => {
                ensure!(
                    !s.move_tool_pending() && !s.tool_selection_pending(),
                    "button budget overlaps tool callback"
                );
                m.history
                    .budgets
                    .0
                    .get(&self.budget)
                    .context("button budget owner absent")?;
                if let Some(balance) = a.pending() {
                    ensure!(
                        s.host
                            .presentation
                            .notifications
                            .as_ref()
                            .is_some_and(
                                |n| n.dispatch.root_message() == Some((0xda, balance as u32))
                            ),
                        "button lost DA child"
                    );
                } else {
                    ensure!(a.complete(), "button budget was not entered");
                }
            }
            Some(Child::ClearProducts | Child::Selection(_)) | None => ensure!(
                !s.move_tool_pending() && !s.tool_selection_pending(),
                "button has unowned tool work"
            ),
        }
        Ok(())
    }
    fn start_move(&mut self, s: &mut Session, op: Operation, after: Phase) -> Result<()> {
        match op {
            Operation::Motion(input) => s.begin_move_tool_motion(input)?,
            Operation::Commit => s.begin_move_tool_commit()?,
            Operation::Pickup(p) => s.begin_move_tool_pickup(p)?,
            Operation::Preview => s.begin_move_tool_preview()?,
            Operation::Reset => s.begin_move_tool_cleanup(cleanup::Kind::Reset)?,
            Operation::Cursor => s.begin_move_tool_cleanup(cleanup::Kind::Cursor)?,
            Operation::Cancel => s.begin_move_tool_cleanup(cleanup::Kind::Deactivate)?,
        }
        s.host
            .presentation
            .move_tool
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .unwrap()
            .button_owner = Some(op);
        self.child = Some(Child::Move(op));
        self.phase = after;
        Ok(())
    }
    fn advance(&mut self, s: &mut Session) -> Result<Progress> {
        if let Some(child) = &mut self.child {
            match child {
                Child::Move(_) => match s.advance_move_tool(1)?.progress {
                    super::Progress::Pending => return Ok(Progress::Pending),
                    super::Progress::Service(r) => return Ok(Progress::Service(Service::Move(r))),
                    super::Progress::Complete => s.finish_move_tool_cleanup()?,
                },
                Child::Selector => match s.advance_tool_selection(1)? {
                    tool_selection::Progress::Pending => return Ok(Progress::Pending),
                    tool_selection::Progress::Service(r) => {
                        return Ok(Progress::Service(Service::Tool(r)));
                    }
                    tool_selection::Progress::Complete => s.finish_tool_selection()?,
                },
                Child::Budget(a) => {
                    if let Some(balance) = a.pending() {
                        match s.advance_house_notification(s.retained_camera_input()?, 1)? {
                            notifications::Progress::Service(r) => {
                                return Ok(Progress::Service(Service::Notification(r)));
                            }
                            notifications::Progress::Pending => return Ok(Progress::Pending),
                            //This is the dispatch return value, not its message
                            //ID. validate() binds the exact DA root and payload.
                            notifications::Progress::Complete(_) => (),
                        }
                        a.acknowledge(balance).map_err(anyhow::Error::msg)?;
                    }
                    ensure!(a.complete(), "button budget incomplete");
                    s.host
                        .presentation
                        .move_tool
                        .as_mut()
                        .unwrap()
                        .history
                        .budgets
                        .0
                        .get_mut(&self.budget)
                        .unwrap()
                        .pending = 0;
                }
                Child::ClearProducts => return Ok(Progress::Service(Service::ClearProducts)),
                Child::Selection(r) => return Ok(Progress::Service(Service::Selection(*r))),
            }
            self.child = None;
            return Ok(Progress::Pending);
        }
        match self.phase {
            Phase::Entry => {
                if matches!(self.input.kind, Kind::Down | Kind::Cancel) {
                    s.host
                        .presentation
                        .move_tool
                        .as_mut()
                        .unwrap()
                        .button_fields
                        .hint = None;
                }
                let (op, after) = match self.input.kind {
                    Kind::Down => (Operation::Motion(self.input.pointer), Phase::Down),
                    Kind::Up => (Operation::Motion(self.input.pointer), Phase::Up),
                    Kind::Cancel => (Operation::Cancel, Phase::Complete),
                };
                self.start_move(s, op, after)?;
            }
            Phase::Down => {
                let m = s.host.presentation.move_tool.as_mut().unwrap();
                for b in [0x81, 0x89, 0x88, 0x79] {
                    m.tool.set_byte(b, 0);
                }
                m.tool.set_byte(0x74, 1);
                m.button_fields.down = self.input.pointer.screen.map(i32::from);
                m.tool.result = -1;
                m.tool.set_byte(0x7b, 1);
                if m.tool.held.is_none() {
                    let picked = self.input.pointer.picked;
                    let cheat = m.history.placement_cheat;
                    if picked != 0 && s.simulation.vm.world.objects.contains_key(&picked) {
                        if sims_sim::lot_cursor::permission::check(&mut s.simulation, picked, cheat)
                            .map_err(anyhow::Error::msg)?
                        {
                            self.start_move(
                                s,
                                Operation::Pickup(self.input.pointer),
                                Phase::Picked,
                            )?;
                            return Ok(Progress::Pending);
                        }
                        s.host
                            .presentation
                            .move_tool
                            .as_mut()
                            .unwrap()
                            .button_fields
                            .hint = Some(Hint::Placement(
                            s.simulation.native_lot.as_ref().unwrap().diagnostics.error,
                        ));
                    }
                    self.start_move(s, Operation::Cursor, Phase::Complete)?;
                    return Ok(Progress::Pending);
                }
                let h = m.tool.held.unwrap();

                let cursor = s.simulation.vm.world.objects
                    [&id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?]
                    .cursor
                    .as_ref()
                    .unwrap();
                if cursor.underlying.is_some() {
                    m.tool.set_byte(0x81, 1);
                    if cursor.valid == 0 {
                        m.button_fields.hint = Some(Hint::Placement(
                            s.simulation.native_lot.as_ref().unwrap().diagnostics.error,
                        ));
                    }
                }
                self.start_move(s, Operation::Cursor, Phase::Complete)?;
            }
            Phase::Picked => {
                let m = s.host.presentation.move_tool.as_mut().unwrap();
                m.button_fields.last = m.button_fields.down;
                m.tool.set_byte(0x89, u8::from(m.tool.byte(0x80) == 0));
                if m.tool.byte(0x80) != 0 {
                    m.tool.set_byte(0x81, 1);
                }
                m.tool.result = -1;
                m.tool.set_byte(0x98, 0);
                for b in [0x7b, 0x97, 0x95, 0x96] {
                    m.tool.set_byte(b, 1);
                }
                m.preview_fields.as_mut().unwrap().points = [-16; 4];
                if let Some(h) = m.tool.held {
                    let held = id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
                    if let Some(object) = s.simulation.vm.world.objects[&held]
                        .cursor
                        .as_ref()
                        .unwrap()
                        .underlying
                        && m.tool.tool.flags[0] == 0
                    {
                        m.tool.set_byte(0x98, 1);
                        let request = pickup::Request::Select {
                            object,
                            definition: m
                                .tool
                                .tool
                                .definition
                                .context("picked definition absent")?,
                            price: pickup::resale(
                                s,
                                id(&s.simulation.vm.world, object).map_err(anyhow::Error::msg)?,
                            )? as i16 as i32,
                            move_allowed: true,
                            sell_allowed: s
                                .host
                                .presentation
                                .move_tool
                                .as_ref()
                                .unwrap()
                                .tool
                                .tool
                                .flags[3]
                                != 0,
                        };
                        self.child = Some(Child::Selection(request));
                    }
                }
                self.phase = Phase::PickupSelected;
            }
            Phase::PickupSelected => {
                s.host
                    .presentation
                    .move_tool
                    .as_mut()
                    .unwrap()
                    .button_fields
                    .picked = self.input.pointer.picked;
                self.start_move(s, Operation::Cursor, Phase::Complete)?;
            }
            Phase::Up => {
                let m = s.host.presentation.move_tool.as_mut().unwrap();
                m.tool.set_byte(0x79, 0);
                if m.tool.held.is_none() {
                    self.phase = Phase::Tail;
                    return Ok(Progress::Pending);
                }
                m.tool.set_byte(0x74, 0);
                m.tool.set_byte(0x7b, 0);
                let point = self.input.pointer.screen.map(i32::from);
                let moved = distance(point, m.button_fields.last) >= 4;
                m.tool.set_byte(0x88, u8::from(moved));
                if moved {
                    m.tool.set_byte(0x81, 1);
                }
                if m.tool.byte(0x81) == 0 {
                    self.phase = Phase::Tail;
                    return Ok(Progress::Pending);
                }
                let m = s.host.presentation.move_tool.take().unwrap();
                let price = preview::price(s, &m);
                s.host.presentation.move_tool = Some(m);
                self.cost = price?;
                let balance = funds(s)?;
                let m = s.host.presentation.move_tool.as_mut().unwrap();
                self.budget = m
                    .preview_fields
                    .as_ref()
                    .context("button budget fields absent")?
                    .budget;
                let pending = m
                    .history
                    .budgets
                    .0
                    .get(&self.budget)
                    .context("button budget absent")?
                    .pending;
                let affordable = (m.tool.value90 as i32).wrapping_add(pending) <= balance;
                let h = m.tool.held.unwrap();
                let cursor = s.simulation.vm.world.objects
                    [&id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?]
                    .cursor
                    .as_ref()
                    .unwrap();
                if !affordable || cursor.underlying.is_none() || cursor.valid == 0 {
                    if distance(point, m.button_fields.down) > 4 {
                        m.tool.tool.position = [0, 0];
                    }
                    m.button_fields.last = point;
                    m.tool.set_byte(0x81, 1);
                    m.tool.set_byte(0x80, 0);
                    m.button_fields.hint = None;
                    if !affordable {
                        s.simulation.native_lot.as_mut().unwrap().diagnostics.error = 21;
                        m.button_fields.hint = Some(Hint::Funds);
                    }
                    self.phase = Phase::Tail;
                } else {
                    self.start_move(s, Operation::Commit, Phase::Committed)?;
                }
            }
            Phase::Committed => self.start_move(s, Operation::Reset, Phase::Reset)?,
            Phase::Reset => {
                let m = s.host.presentation.move_tool.as_mut().unwrap();
                let b = m
                    .history
                    .budgets
                    .0
                    .get_mut(&self.budget)
                    .context("button budget absent")?;
                b.pending = b.pending.wrapping_add(m.tool.value90 as i32);
                if b.pending != 0 {
                    m.button_fields.hint = Some(Hint::Spent(b.pending));
                    self.phase = Phase::Budget;
                } else {
                    self.phase = Phase::Charged;
                }
            }
            Phase::Budget => {
                s.check_house_notification(0xda)?;
                let n = s
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .context("button listeners absent")?;
                let registry = n.dispatch.registry.clone();
                let fields = n.dispatch.fields.clone();
                sims_vm::commander::house::dispatch::Dispatch::begin(
                    registry.clone(),
                    fields.clone(),
                    0xda,
                    0,
                )
                .map_err(anyhow::Error::msg)?;
                let b = s
                    .host
                    .presentation
                    .move_tool
                    .as_ref()
                    .unwrap()
                    .history
                    .budgets
                    .0[&self.budget];
                let mut a = Application::new(b.category, b.pending);
                if let Some(balance) = s
                    .simulation
                    .advance_object_budget(&mut a)
                    .map_err(anyhow::Error::msg)?
                {
                    s.install_house_notification(registry, fields, 0xda, balance as u32)?;
                }
                self.child = Some(Child::Budget(a));
                self.phase = Phase::Charged;
            }
            Phase::Charged => {
                let m = s.host.presentation.move_tool.as_mut().unwrap();
                m.tool.value90 = 0;
                m.tool.set_byte(0x81, 0);
                self.phase = if m.tool.tool.flags[0] != 0 {
                    Phase::Repeat
                } else {
                    Phase::Tail
                };
            }
            Phase::Repeat => {
                let repeat = self.input.shift
                    || s.host
                        .presentation
                        .move_tool
                        .as_ref()
                        .unwrap()
                        .tool
                        .tool
                        .flags[1]
                        != 0;
                let balance = funds(s)?;
                if repeat && self.cost <= balance {
                    self.start_move(s, Operation::Preview, Phase::Repeated)?;
                } else {
                    let controls = &mut s
                        .host
                        .presentation
                        .transport
                        .as_mut()
                        .context("button CPState absent")?
                        .controls;
                    if controls.mode == 1 {
                        controls.buy.selection_aux = u32::MAX;
                        controls.buy.selected_object = 0;
                        s.begin_buy_tool(3)?;
                        s.host
                            .presentation
                            .tool_selection
                            .as_mut()
                            .unwrap()
                            .claim_button()?;
                        self.child = Some(Child::Selector);
                        self.phase = Phase::Selected;
                    } else {
                        self.phase = Phase::SelectionCleared;
                    }
                }
            }
            Phase::Repeated => {
                let m = s.host.presentation.move_tool.as_mut().unwrap();
                m.tool.set_byte(0x97, 1);
                m.tool.set_byte(0x98, 0);
                self.phase = Phase::Tail;
            }
            Phase::Selected => {
                //50a8e1/50670d deselect actual product widgets after selector3.
                self.child = Some(Child::ClearProducts);
                self.phase = Phase::SelectionCleared;
            }
            Phase::SelectionCleared => {
                s.host
                    .presentation
                    .move_tool
                    .as_mut()
                    .unwrap()
                    .tool
                    .tool
                    .flags[0] = 0;
                self.phase = Phase::Tail;
            }
            Phase::Tail => {
                self.start_move(s, Operation::Motion(self.input.pointer), Phase::Complete)?
            }
            Phase::Complete => return Ok(Progress::Complete),
        }
        Ok(Progress::Pending)
    }
}
impl Session {
    pub fn move_button_pending(&self) -> bool {
        self.host.presentation.move_button.is_some()
    }
    pub(in crate::original_session) fn validate_move_button_owner(&self) -> Result<()> {
        if let Some(b) = &self.host.presentation.move_button {
            b.validate(self)?;
        }
        if let Some(op) = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .and_then(|m| m.call.as_ref()?.button_owner)
        {
            ensure!(
                self.host
                    .presentation
                    .move_button
                    .as_ref()
                    .is_some_and(|b| b.child == Some(Child::Move(op))),
                "orphaned button Move child"
            );
        }
        if self
            .host
            .presentation
            .tool_selection
            .as_ref()
            .is_some_and(|t| t.button_owned())
        {
            ensure!(
                self.host
                    .presentation
                    .move_button
                    .as_ref()
                    .is_some_and(|b| b.child == Some(Child::Selector)),
                "orphaned button selector"
            );
        }
        Ok(())
    }
    pub fn begin_move_button(&mut self, input: Input) -> Result<()> {
        self.check_move_tool_entry()?;
        let m = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("button Move absent")?;
        ensure!(
            input.kind == Kind::Cancel || matches!(m.tool.tool.mode, 1 | 2),
            "button currently requires ordinary furniture"
        );
        if input.kind != Kind::Cancel && m.tool.held.is_none() {
            if let Some(o) = self.simulation.vm.world.objects.get(&input.pointer.picked) {
                ensure!(
                    o.person.is_none() && o.cursor.is_none(),
                    "this object requires a different pickup service"
                );
                let d = o.definition.as_ref().context("pickup definition absent")?;
                ensure!(
                    sims_vm::construction::definition_words(&self.program, d)
                        .map_err(anyhow::Error::msg)?[69]
                        == 0,
                    "special furniture pickup is not supported yet"
                );
            } else {
                let floor = self
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .context("pickup floor absent")?
                    .view
                    .floor
                    .level;
                ensure!(
                    floor <= 1,
                    "empty upper-floor pickup requires its original floor service"
                );
            }
        }
        if input.kind != Kind::Cancel
            && let Some(h) = m.tool.held
        {
            let c = self.simulation.vm.world.objects
                [&id(&self.simulation.vm.world, h).map_err(anyhow::Error::msg)?]
                .cursor
                .as_ref()
                .unwrap();
            if let Some(h) = c.underlying {
                let obj = &self.simulation.vm.world.objects
                    [&id(&self.simulation.vm.world, h).map_err(anyhow::Error::msg)?];
                let definition = obj
                    .definition
                    .as_ref()
                    .context("button definition absent")?;
                ensure!(
                    sims_vm::construction::definition_words(&self.program, definition)
                        .map_err(anyhow::Error::msg)?[69]
                        != 6,
                    "lighting purchase requires original house91 delivery"
                );
            }
        }
        self.host.presentation.move_button = Some(State {
            tick: self.simulation.vm.tick,
            input,
            phase: Phase::Entry,
            child: None,
            cost: 0,
            budget: 0,
        });
        self.validate_move_button_owner()
    }
    pub fn advance_move_button(&mut self) -> Result<Progress> {
        self.validate_move_button_owner()?;
        let mut b = self
            .host
            .presentation
            .move_button
            .take()
            .context("button caller absent")?;
        let result = b.advance(self);
        self.host.presentation.move_button = Some(b);
        self.validate_move_button_owner()?;
        result
    }
    pub fn acknowledge_move_button_selection(&mut self, r: pickup::Request) -> Result<()> {
        let b = self
            .host
            .presentation
            .move_button
            .as_mut()
            .context("button selection caller absent")?;
        ensure!(
            b.child == Some(Child::Selection(r)),
            "button selection reply differs"
        );
        b.child = None;
        Ok(())
    }
    pub fn acknowledge_move_button_products(&mut self) -> Result<()> {
        let b = self
            .host
            .presentation
            .move_button
            .as_mut()
            .context("button caller absent")?;
        ensure!(
            b.child == Some(Child::ClearProducts),
            "button catalog clear has no caller"
        );
        b.child = None;
        Ok(())
    }
    pub fn finish_move_button(&mut self) -> Result<()> {
        self.validate_move_button_owner()?;
        ensure!(
            self.host
                .presentation
                .move_button
                .as_ref()
                .is_some_and(|b| b.phase == Phase::Complete && b.child.is_none()),
            "button has not returned"
        );
        self.host.presentation.move_button = None;
        Ok(())
    }
}
