//! Original4cefce tool replacement and4cef9b indexed selection. Indexed
//! selection activates the new tool both before and after replacing the old
//! one. The listing, rather than the decompiler's misidentified stack argument,
//! establishes that both activations receive the same second argument.
use serde::{Deserialize, Serialize};

///48e21b changes cObjPickerTool+3c only for the signed low words0 and1.
///None preserves the constructor's undefined value until its first write.
pub fn activate_picker(mode: &mut Option<i32>, argument: u32) {
    match argument as i16 {
        0 => *mode = Some(0),
        1 => *mode = Some(2),
        _ => (),
    }
}

///48c91b calls584645(0), which writes the drawing-mode byte at6f6bc8.
pub const PICKER_DEACTIVATED_DRAWING_MODE: u8 = 0;

/// 4847cf reads a signed *word* argument before checking the twelve-slot table.
/// Retained null slots remain null; callers decide whether that is a fault.
pub fn lookup(slots: &[Option<u32>; 12], index: i32) -> Option<u32> {
    let index = index as i16;
    if (0..12).contains(&index) {
        slots[index as usize].filter(|&id| id != 0)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub active: Option<u32>,
    pub argument: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///4847cf(index), on Root+18.
    Lookup(i32),
    ///Old tool virtual+60.
    Deactivate(u32),
    ///Tool virtual+30(argument).
    Activate { tool: u32, argument: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Tool(u32),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Lookup,
    Before,
    Old,
    Publish,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    index: Option<i32>,
    tool: Option<u32>,
    argument: u32,
    stage: Stage,
    pending: Option<Request>,
}
impl Change {
    pub fn selects(&self, index: i32, argument: u32) -> bool {
        self.index == Some(index) && self.argument == argument
    }
    pub fn replaces(&self, tool: Option<u32>, argument: u32) -> bool {
        self.index.is_none() && self.tool == tool && self.argument == argument
    }
    pub fn replace(tool: Option<u32>, argument: u32) -> Result<Self, String> {
        if tool == Some(0) {
            return Err("null tool must use None".into());
        }
        Ok(Self {
            index: None,
            tool,
            argument,
            stage: Stage::Old,
            pending: None,
        })
    }
    pub fn select(index: i32, argument: u32) -> Self {
        Self {
            index: Some(index),
            tool: None,
            argument,
            stage: Stage::Lookup,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = self.tool != Some(0)
            && match (&self.pending, self.stage) {
                (None, Stage::Lookup | Stage::Before) => self.index.is_some(),
                (None, _) => self.index.is_none() || self.tool.is_some(),
                (Some(Request::Lookup(id)), Stage::Before) => {
                    self.index == Some(*id) && self.tool.is_none()
                }
                (Some(Request::Activate { tool, argument }), Stage::Old) => {
                    self.index.is_some() && self.tool == Some(*tool) && self.argument == *argument
                }
                (Some(Request::Deactivate(tool)), Stage::Publish) => *tool != 0,
                (Some(Request::Activate { tool, argument }), Stage::Complete) => {
                    self.tool == Some(*tool) && self.argument == *argument
                }
                _ => false,
            };
        if valid {
            Ok(())
        } else {
            Err("tool replacement lost its callback".into())
        }
    }
    pub fn acknowledge(&mut self, request: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("tool replacement reply differs".into());
        }
        match (request, reply) {
            (Request::Lookup(_), Reply::Tool(tool)) if tool != 0 => self.tool = Some(tool),
            (Request::Deactivate(_) | Request::Activate { .. }, Reply::Done) => (),
            _ => return Err("tool replacement reply kind differs or tool is null".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        let request = loop {
            match self.stage {
                Stage::Lookup => {
                    self.stage = Stage::Before;
                    break Request::Lookup(self.index.expect("validated index"));
                }
                Stage::Before => {
                    let tool = self.tool.ok_or("indexed tool lookup has no result")?;
                    self.stage = Stage::Old;
                    break Request::Activate {
                        tool,
                        argument: self.argument,
                    };
                }
                Stage::Old => {
                    if fields.active == Some(0) {
                        return Err("invalid retained active tool".into());
                    }
                    self.stage = Stage::Publish;
                    if let Some(tool) = fields.active {
                        break Request::Deactivate(tool);
                    }
                }
                Stage::Publish => {
                    fields.active = self.tool;
                    fields.argument = self.argument;
                    self.stage = Stage::Complete;
                    if let Some(tool) = self.tool {
                        break Request::Activate {
                            tool,
                            argument: self.argument,
                        };
                    }
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request.clone());
        Ok(Some(request))
    }
}
