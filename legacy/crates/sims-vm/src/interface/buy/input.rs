//!50633e category/page input. Tool callbacks execute between category prefix
//!writes and dirty400;507086 publishes selection after both specific setters.
use super::{
    Fields,
    category::{Axis, Change},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    Category(u8),
    Back,
    Previous,
    Next,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Tool { index: i32, argument: u32 },
    CategorySelected(Option<u8>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Row,
    Column,
    Style,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Navigation {
    event: Event,
    stage: Stage,
    catalog: Option<u8>,
    selected: Option<u8>,
    child: Option<Change>,
    pending: Option<Request>,
}
impl Navigation {
    pub fn new(event: Event) -> Result<Self, String> {
        if matches!(event, Event::Category(i) if i>=8) {
            return Err("Buy category widget absent".into());
        }
        Ok(Self {
            event,
            stage: Stage::Entry,
            catalog: None,
            selected: None,
            child: None,
            pending: None,
        })
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.event, Event::Category(i) if i>=8)
            || self.catalog.is_some_and(|i| i > 6)
            || self.selected.is_some_and(|i| i >= 8)
        {
            return Err("Buy navigation fields invalid".into());
        }
        if let Some(c) = &self.child {
            c.validate()?;
            if !matches!(self.stage, Stage::Row | Stage::Column) || self.catalog.is_none() {
                return Err("Buy category child lost navigation".into());
            }
        }
        if let Some(r) = self.pending {
            match r {
                Request::Tool { index, argument }
                    if self.child.as_ref().and_then(Change::request)
                        == Some(super::category::Request { index, argument }) =>
                {
                    ()
                }
                Request::CategorySelected(_)
                    if self.stage == Stage::Complete && self.child.is_none() =>
                {
                    ()
                }
                _ => return Err("Buy input lost child request".into()),
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("Buy input reply differs".into());
        }
        if let Request::Tool { index, argument } = request {
            self.child
                .as_mut()
                .unwrap()
                .acknowledge(super::category::Request { index, argument })?;
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
            match self.stage {
                Stage::Entry => match self.event {
                    Event::Previous | Event::Next => {
                        fields.set_page(if self.event == Event::Next {
                            fields.selected_table.wrapping_add(1)
                        } else {
                            fields.selected_table.wrapping_sub(1)
                        })?;
                        self.stage = Stage::Complete;
                    }
                    Event::Back => {
                        if fields.subcatalog == 1 {
                            fields.set_subcatalog(0)?;
                            self.stage = Stage::Style;
                        } else {
                            self.stage = Stage::Complete;
                        }
                    }
                    Event::Category(i) => {
                        self.catalog = (0..7)
                            .contains(&fields.catalog)
                            .then_some(fields.catalog as u8);
                        if fields.subcatalog == 0 {
                            fields.set_subcatalog(1)?;
                            self.selected = Some(0);
                            self.stage = Stage::Row;
                            self.child = self
                                .catalog
                                .map(|c| Change::new(c, Axis::Row, i.into()))
                                .transpose()?;
                        } else {
                            self.selected = Some(i);
                            self.stage = if fields.subcatalog == 1 {
                                Stage::Column
                            } else {
                                Stage::Style
                            };
                            if self.stage == Stage::Column {
                                self.child = self
                                    .catalog
                                    .map(|c| Change::new(c, Axis::Column, i.into()))
                                    .transpose()?;
                            }
                        }
                    }
                },
                Stage::Row | Stage::Column => {
                    if let Some(c) = &mut self.child {
                        if let Some(r) = c.advance(fields)? {
                            self.pending = Some(Request::Tool {
                                index: r.index,
                                argument: r.argument,
                            });
                            break;
                        }
                        if !c.complete() {
                            return Err("Buy category returned early".into());
                        }
                    }
                    self.child = None;
                    if self.stage == Stage::Row {
                        self.stage = Stage::Column;
                        self.child = self
                            .catalog
                            .map(|c| Change::new(c, Axis::Column, 0))
                            .transpose()?;
                    } else {
                        self.stage = Stage::Style;
                    }
                }
                Stage::Style => {
                    self.stage = Stage::Complete;
                    self.pending = Some(Request::CategorySelected(if fields.subcatalog == 1 {
                        self.selected
                    } else {
                        None
                    }));
                    break;
                }
                Stage::Complete => break,
            }
        }
        Ok(self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::state::State;
    #[test]
    fn navigation_keeps_captured_setter_pair_and_live_style_after_tool_return() {
        let mut f = State::constructed(760).buy;
        f.catalog = 1;
        f.subcatalog = 0;
        f.indices[1] = [2, 4];
        let mut c = Navigation::new(Event::Category(3)).unwrap();
        let tool = Request::Tool {
            index: 7,
            argument: u32::MAX,
        };
        assert_eq!(c.advance(&mut f).unwrap(), Some(tool));
        assert_eq!(f.indices[1], [3, 0]);
        assert_eq!(f.subcatalog, 1);
        c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        // A real nested callback can change the active catalog and column.
        f.catalog = 2;
        f.indices[1][1] = 5;
        f.dirty = 0;
        c.acknowledge(tool).unwrap();
        assert_eq!(c.advance(&mut f).unwrap(), Some(tool));
        assert_eq!(f.indices[1], [3, 0]);
        assert_eq!(f.catalog, 2);
        f.subcatalog = 0;
        c.acknowledge(tool).unwrap();
        assert_eq!(
            c.advance(&mut f).unwrap(),
            Some(Request::CategorySelected(None))
        );
        c.acknowledge(Request::CategorySelected(None)).unwrap();
        assert!(c.complete());
    }
    #[test]
    fn pages_keep_product_selection_and_back_only_resets_the_open_subcatalog() {
        let mut f = State::constructed(760).buy;
        f.subcatalog = 1;
        f.selected_object = 42;
        f.selection_aux = 2;
        f.dirty = 0;
        let mut c = Navigation::new(Event::Previous).unwrap();
        assert_eq!(c.advance(&mut f).unwrap(), None);
        assert_eq!(f.selected_table, u32::MAX);
        assert_eq!(f.selected_object, 42);
        let mut c = Navigation::new(Event::Back).unwrap();
        assert_eq!(
            c.advance(&mut f).unwrap(),
            Some(Request::CategorySelected(None))
        );
        assert_eq!(f.selected_object, 0);
        assert_eq!(f.selected_table, 0);
        c.acknowledge(Request::CategorySelected(None)).unwrap();
        let before = f.clone();
        let mut c = Navigation::new(Event::Back).unwrap();
        assert_eq!(c.advance(&mut f).unwrap(), None);
        assert_eq!(f, before);
    }
}
