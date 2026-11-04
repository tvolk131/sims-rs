//!4ceff9: select a definition by GUID, retaining its ordinal across reset and
//! replacement callbacks, then publish487ae6's special index or that ordinal.
use crate::interface::tool::Fields;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Find(u32),
    LookupTool,
    Ordinal(u32),
    Reset(u32),
    Replace { tool: u32, argument: u32 },
    Classify { tool: u32, definition: u32 },
    SelectDefault,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Number(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Found,
    Tool,
    Ordinal,
    Reset,
    Replace,
    Classify,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    guid: u32,
    stage: Stage,
    definition: u32,
    tool: u32,
    ordinal: u32,
    answer: u32,
    pending: Option<Request>,
}
impl Selection {
    pub fn guid(&self) -> u32 {
        self.guid
    }
    pub fn new(guid: u32) -> Self {
        Self {
            guid,
            stage: Stage::Entry,
            definition: 0,
            tool: 0,
            ordinal: 0,
            answer: 0,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        let owners = match self.stage {
            Stage::Entry | Stage::Found => self.definition == 0 && self.tool == 0,
            Stage::Tool => self.definition != 0 && self.tool == 0,
            Stage::Ordinal | Stage::Reset | Stage::Replace | Stage::Classify => {
                self.definition != 0 && self.tool != 0
            }
            Stage::Complete => (self.definition == 0) == (self.tool == 0),
        };
        let valid = match (self.stage, self.pending) {
            (_, None) => true,
            (Stage::Found, Some(Request::Find(g))) => g == self.guid,
            (Stage::Tool, Some(Request::LookupTool)) => self.definition != 0,
            (Stage::Ordinal, Some(Request::Ordinal(d))) => {
                d == self.definition && d != 0 && self.tool != 0
            }
            (Stage::Reset, Some(Request::Reset(t))) => t == self.tool && t != 0,
            (Stage::Replace, Some(Request::Replace { tool, argument })) => {
                tool == self.tool && tool != 0 && argument == self.ordinal
            }
            (Stage::Classify, Some(Request::Classify { tool, definition })) => {
                tool == self.tool && definition == self.definition && tool != 0 && definition != 0
            }
            (Stage::Complete, Some(Request::SelectDefault)) => self.definition == 0,
            _ => false,
        };
        if owners && valid {
            Ok(())
        } else {
            Err("Buy definition handoff lost its child".into())
        }
    }
    pub fn acknowledge(&mut self, r: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("Buy definition reply differs".into());
        }
        match (r, reply) {
            (
                Request::Find(_)
                | Request::LookupTool
                | Request::Ordinal(_)
                | Request::Classify { .. },
                Reply::Number(n),
            ) => self.answer = n,
            (Request::Reset(_) | Request::Replace { .. } | Request::SelectDefault, Reply::Done) => {
                ()
            }
            _ => return Err("Buy definition reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let r = match self.stage {
            Stage::Entry => {
                self.stage = Stage::Found;
                Request::Find(self.guid)
            }
            Stage::Found => {
                self.definition = self.answer;
                if self.definition == 0 {
                    self.stage = Stage::Complete;
                    Request::SelectDefault
                } else {
                    self.stage = Stage::Tool;
                    Request::LookupTool
                }
            }
            Stage::Tool => {
                if self.answer == 0 {
                    return Err("Buy definition tool7 absent".into());
                }
                self.tool = self.answer;
                self.stage = Stage::Ordinal;
                Request::Ordinal(self.definition)
            }
            Stage::Ordinal => {
                self.ordinal = self.answer;
                self.stage = Stage::Reset;
                Request::Reset(self.tool)
            }
            Stage::Reset => {
                self.stage = Stage::Replace;
                Request::Replace {
                    tool: self.tool,
                    argument: self.ordinal,
                }
            }
            Stage::Replace => {
                self.stage = Stage::Classify;
                Request::Classify {
                    tool: self.tool,
                    definition: self.definition,
                }
            }
            Stage::Classify => {
                fields.argument = if self.answer != 0 {
                    self.answer
                } else {
                    self.ordinal
                };
                self.stage = Stage::Complete;
                return Ok(None);
            }
            Stage::Complete => return Ok(None),
        };
        self.pending = Some(r);
        Ok(Some(r))
    }
}
