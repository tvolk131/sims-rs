//!5065c8 product click, composed with the real4cf821 ->4cf360 tool child.
//!The page is read afresh after callbacks; affordability and widget identity
//!are captured before the tool call, while the details product is read after it.
use super::Fields;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    End,
    Empty,
    Item { product: u32, widget: Option<u32> },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Read(u32),
    Cost(u32),
    BuyTool,
    ///Inspect the live details window; refresh this live page ordinal if open.
    DetailsIfOpen(u32),
    Widget {
        widget: u32,
        selected: bool,
    },
    ErrorFeedback,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Slot(Slot),
    Cost { funds: i32, price: i32 },
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Visit,
    Read,
    Cost,
    GotCost,
    Select,
    AfterTool,
    AfterDetails,
    Other,
    Error,
    Next,
    Clear,
    AfterClear,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Click {
    product: u32,
    index: u32,
    any_selected: bool,
    widget: Option<u32>,
    affordable: Option<bool>,
    stage: Stage,
    pending: Option<Request>,
}
impl Click {
    pub fn new(product: u32) -> Self {
        Self {
            product,
            index: 0,
            any_selected: false,
            widget: None,
            affordable: None,
            stage: Stage::Visit,
            pending: None,
        }
    }
    pub fn product(&self) -> u32 {
        self.product
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = self.widget != Some(0)
            && match self.pending {
                None => true,
                Some(Request::Read(i)) => self.stage == Stage::Read && i == self.index,
                Some(Request::Cost(p)) => self.stage == Stage::GotCost && p == self.product,
                Some(Request::BuyTool) => {
                    matches!(self.stage, Stage::AfterTool | Stage::AfterClear)
                }
                Some(Request::DetailsIfOpen(i)) => {
                    self.stage == Stage::AfterDetails && i == self.index
                }
                Some(Request::Widget { widget, .. }) => {
                    self.widget == Some(widget) && matches!(self.stage, Stage::Next | Stage::Error)
                }
                Some(Request::ErrorFeedback) => self.stage == Stage::Next,
            };
        if !valid
            || (matches!(
                self.stage,
                Stage::Select | Stage::AfterTool | Stage::AfterDetails | Stage::Error
            ) && self.affordable.is_none())
        {
            return Err("Buy product input lost its callback".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("Buy product reply differs".into());
        }
        match (request, reply) {
            (Request::Read(_), Reply::Slot(slot)) => {
                self.widget = None;
                self.affordable = None;
                self.stage = match slot {
                    Slot::End => {
                        if self.any_selected {
                            Stage::Complete
                        } else {
                            Stage::Clear
                        }
                    }
                    Slot::Empty => Stage::Next,
                    Slot::Item { product, widget } => {
                        if product == 0 || widget == Some(0) {
                            return Err("null Buy item/widget must be explicit".into());
                        }
                        self.widget = widget;
                        if product == self.product || widget.is_none() {
                            Stage::Cost
                        } else {
                            Stage::Other
                        }
                    }
                };
            }
            (Request::Cost(_), Reply::Cost { funds, price }) => {
                self.affordable = Some(price <= funds);
                self.stage = Stage::Select;
            }
            (
                Request::BuyTool
                | Request::DetailsIfOpen(_)
                | Request::Widget { .. }
                | Request::ErrorFeedback,
                Reply::Done,
            ) => (),
            _ => return Err("Buy product reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            self.pending = match self.stage {
                Stage::Visit => {
                    self.stage = Stage::Read;
                    Some(Request::Read(self.index))
                }
                Stage::Cost => {
                    self.stage = Stage::GotCost;
                    Some(Request::Cost(self.product))
                }
                Stage::Select => {
                    fields.selection_aux = self.index;
                    fields.selected_object = self.product;
                    self.stage = Stage::AfterTool;
                    Some(Request::BuyTool)
                }
                Stage::AfterTool => {
                    fields.dirty |= 0x200;
                    self.stage = Stage::AfterDetails;
                    Some(Request::DetailsIfOpen(self.index))
                }
                Stage::AfterDetails => {
                    let selected = self.affordable.ok_or("Buy affordability absent")?;
                    self.any_selected |= selected;
                    self.stage = if selected { Stage::Next } else { Stage::Error };
                    self.widget
                        .map(|widget| Request::Widget { widget, selected })
                }
                Stage::Other => {
                    self.stage = Stage::Next;
                    self.widget.map(|widget| Request::Widget {
                        widget,
                        selected: false,
                    })
                }
                Stage::Error => {
                    self.stage = Stage::Next;
                    Some(Request::ErrorFeedback)
                }
                Stage::Next => {
                    self.index = self.index.wrapping_add(1);
                    self.stage = Stage::Visit;
                    None
                }
                Stage::Clear => {
                    fields.selection_aux = u32::MAX;
                    fields.selected_object = 0;
                    self.stage = Stage::AfterClear;
                    Some(Request::BuyTool)
                }
                Stage::AfterClear => {
                    fields.dirty |= 0x200;
                    self.stage = Stage::Complete;
                    None
                }
                Stage::Complete => return Ok(None),
                Stage::Read | Stage::GotCost => {
                    return Err("Buy product getter lost its reply".into());
                }
            };
            if self.pending.is_some() {
                return Ok(self.pending);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::state::State;
    #[test]
    fn unaffordable_product_still_selects_then_clears_and_keeps_captured_cost() {
        let mut fields = State::constructed(760).buy;
        fields.dirty = 0;
        let mut c = Click::new(10);
        let mut tools = vec![];
        let mut styles = vec![];
        let mut errors = 0;
        for _ in 0..30 {
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            let Some(r) = c.advance(&mut fields).unwrap() else {
                break;
            };
            let reply = match r {
                Request::Read(0) => Reply::Slot(Slot::Item {
                    product: 10,
                    widget: Some(20),
                }),
                Request::Read(_) => Reply::Slot(Slot::End),
                Request::Cost(10) => Reply::Cost {
                    funds: 99,
                    price: 100,
                },
                Request::BuyTool => {
                    tools.push((fields.selection_aux, fields.selected_object, fields.dirty));
                    fields.dirty = 0;
                    Reply::Done
                }
                Request::DetailsIfOpen(0) => {
                    assert_eq!(fields.dirty, 0x200);
                    Reply::Done
                }
                Request::Widget { widget, selected } => {
                    styles.push((widget, selected));
                    Reply::Done
                }
                Request::ErrorFeedback => {
                    errors += 1;
                    Reply::Done
                }
                _ => panic!("unexpected {r:?}"),
            };
            c.acknowledge(r, reply).unwrap();
        }
        assert!(c.complete());
        assert_eq!(tools, vec![(0, 10, 0), (u32::MAX, 0, 0x200)]);
        assert_eq!(styles, vec![(20, false)]);
        assert_eq!(errors, 1);
        assert_eq!(fields.dirty, 0x200);
    }
    #[test]
    fn missing_widget_selects_clicked_product_and_reloads_page_after_child() {
        let mut fields = State::constructed(760).buy;
        let mut c = Click::new(42);
        let mut read = vec![];
        let mut changed = false;
        let mut tools = 0;
        for _ in 0..30 {
            let Some(r) = c.advance(&mut fields).unwrap() else {
                break;
            };
            let reply = match r {
                Request::Read(i) => {
                    read.push(i);
                    Reply::Slot(if changed {
                        Slot::End
                    } else {
                        Slot::Item {
                            product: 9,
                            widget: None,
                        }
                    })
                }
                Request::Cost(42) => Reply::Cost {
                    funds: -1,
                    price: i32::MIN,
                },
                Request::BuyTool => {
                    assert_eq!(fields.selected_object, 42);
                    changed = true;
                    tools += 1;
                    Reply::Done
                }
                Request::DetailsIfOpen(0) => Reply::Done,
                _ => panic!("unexpected {r:?}"),
            };
            c.acknowledge(r, reply).unwrap();
        }
        assert!(c.complete());
        assert_eq!(tools, 1);
        assert_eq!(read, vec![0, 1]);
    }
}
