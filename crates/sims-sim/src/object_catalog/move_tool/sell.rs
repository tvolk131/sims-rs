//!487e18 and the4929fb flush tail. Engine children stay explicit; this kernel
//!does not delete furniture, fabricate history, or acknowledge a notification.
use super::cleanup;
use crate::edit_history::object::budget;
use serde::{Deserialize, Serialize};

pub trait View {
    fn underlying(&self, cursor: u32) -> Result<Option<u32>, String>;
    fn set_payload_cost(&mut self, payload: u32, cost: i32) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Feedback {
    Blocked,
    Delete,
    MoneyBack,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    CanSell(u32),                          // underlying virtual+70
    Placed(u32),                           //432595, on the current cursor
    Detach(u32),                           //cursor virtual+50(1)
    Resale(u32),                           //43219a, on the originally captured underlying object
    Budget { category: i32, amount: i32 }, //46d91c inside4929fb
    Feedback(Feedback),
    Reset,      //tool virtual+18
    Deactivate, //tool virtual+60 (can occur twice)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Bool(bool),
    Price(i16),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Permission,
    CheckPlaced,
    Placed,
    Detach,
    Account,
    Price,
    Calculate,
    Budget,
    BudgetReturned,
    Feedback,
    Blocked,
    EarlyDeactivate,
    Reset,
    Deactivate,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    phase: Phase,
    pending: Option<Request>,
    object: Option<u32>,
    price: i16,
    refund: i32,
}
impl Default for Call {
    fn default() -> Self {
        Self {
            phase: Phase::Entry,
            pending: None,
            object: None,
            price: 0,
            refund: 0,
        }
    }
}
impl Call {
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self.pending {
            None => true,
            Some(Request::CanSell(h)) => self.phase == Phase::Permission && self.object == Some(h),
            Some(Request::Placed(h)) => self.phase == Phase::Placed && h != 0,
            Some(Request::Detach(h)) => self.phase == Phase::Detach && h != 0,
            Some(Request::Resale(h)) => self.phase == Phase::Price && self.object == Some(h),
            Some(Request::Budget { .. }) => self.phase == Phase::Budget,
            Some(Request::Feedback(Feedback::Blocked)) => self.phase == Phase::Blocked,
            Some(Request::Feedback(Feedback::Delete)) => {
                self.phase == Phase::Feedback && self.refund == 0
            }
            Some(Request::Feedback(Feedback::MoneyBack)) => {
                self.phase == Phase::Feedback && self.refund != 0
            }
            Some(Request::Reset) => self.phase == Phase::Reset,
            Some(Request::Deactivate) => {
                matches!(self.phase, Phase::EarlyDeactivate | Phase::Deactivate)
            }
        };
        if !valid
            || self.object == Some(0)
            || (!matches!(self.phase, Phase::Entry | Phase::Blocked | Phase::Complete)
                && self.object.is_none())
        {
            return Err("invalid Move sell continuation".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("Move sell reply lost its caller".into());
        }
        let next = match (request, reply) {
            (Request::CanSell(_), Reply::Bool(ok)) => {
                if ok {
                    Phase::CheckPlaced
                } else {
                    Phase::Blocked
                }
            }
            (Request::Placed(_), Reply::Bool(placed)) => {
                if placed {
                    Phase::Detach
                } else {
                    Phase::Account
                }
            }
            (Request::Detach(_), Reply::Done) => Phase::Account,
            (Request::Resale(_), Reply::Price(price)) => {
                self.price = price;
                Phase::Calculate
            }
            (Request::Budget { .. }, Reply::Done) => Phase::BudgetReturned,
            (Request::Feedback(Feedback::Blocked), Reply::Done) => Phase::Complete,
            (Request::Feedback(_), Reply::Done) => Phase::Reset,
            (Request::Deactivate, Reply::Done) if self.phase == Phase::EarlyDeactivate => {
                Phase::Reset
            }
            (Request::Reset, Reply::Done) => Phase::Deactivate,
            (Request::Deactivate, Reply::Done) => Phase::Complete,
            _ => return Err("Move sell reply kind differs".into()),
        };
        self.pending = None;
        self.phase = next;
        Ok(())
    }
    pub fn advance(
        &mut self,
        state: &mut cleanup::State,
        budget: &mut budget::State,
        view: &mut impl View,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        state.validate()?;
        if self.pending.is_some() || self.complete() {
            return Ok(self.pending);
        }
        let request = loop {
            match self.phase {
                Phase::Entry => {
                    let Some(cursor) = state.held else {
                        self.phase = Phase::Complete;
                        return Ok(None);
                    };
                    self.object = view.underlying(cursor)?;
                    self.phase = if self.object.is_some() {
                        Phase::Permission
                    } else {
                        Phase::Blocked
                    };
                }
                Phase::Permission => break Request::CanSell(self.object.unwrap()),
                Phase::CheckPlaced => {
                    self.phase = if state.held.is_some() {
                        Phase::Placed
                    } else {
                        Phase::Account
                    };
                }
                Phase::Placed => {
                    break Request::Placed(
                        state
                            .held
                            .ok_or("sell cursor vanished before placed query")?,
                    );
                }
                Phase::Detach => {
                    break Request::Detach(state.held.ok_or("sell cursor vanished before detach")?);
                }
                Phase::Account => {
                    self.phase = if state.tool.flags[0] != 0 {
                        Phase::EarlyDeactivate
                    } else {
                        Phase::Price
                    };
                }
                Phase::Price => break Request::Resale(self.object.unwrap()),
                Phase::Calculate => {
                    let price = i32::from(self.price);
                    //The payload and policy bytes are reread after43219a.
                    if let Some(payload) = state.payload {
                        view.set_payload_cost(payload, price)?;
                    }
                    self.refund = if state.tool.flags[3] == 0 {
                        0
                    } else if state.tool.flags[2] != 0 {
                        // FILD signed price; FMUL original f32; FADD0.5;
                        // __ftol truncates toward zero, including negatives.
                        (f64::from(price) * f64::from(f32::from_bits(0x3f4ccccd)) + 0.5).trunc()
                            as i32
                    } else {
                        price
                    };
                    budget.pending = budget.pending.wrapping_sub(self.refund);
                    self.phase = Phase::Budget;
                }
                Phase::Budget => {
                    break Request::Budget {
                        category: budget.category,
                        amount: budget.pending,
                    };
                }
                Phase::BudgetReturned => {
                    //4929fb clears even an adjustment written by its callback.
                    budget.pending = 0;
                    self.phase = Phase::Feedback;
                }
                Phase::Feedback => {
                    break Request::Feedback(if self.refund == 0 {
                        Feedback::Delete
                    } else {
                        Feedback::MoneyBack
                    });
                }
                Phase::Blocked => break Request::Feedback(Feedback::Blocked),
                Phase::EarlyDeactivate | Phase::Deactivate => break Request::Deactivate,
                Phase::Reset => break Request::Reset,
                Phase::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
}
