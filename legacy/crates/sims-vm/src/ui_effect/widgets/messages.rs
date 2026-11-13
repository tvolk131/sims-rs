//! Original5da782: swap the two20-byte-record vectors only when active is
//! empty, advance the cursor before membership/dispatch, clear on exhaustion.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub owner: u32,
    pub words: [u32; 4],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vector {
    pub entries: Vec<Message>,
    pub capacity: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub inbox: Vector,
    pub active: Vector,
    pub cursor: u32,
}
impl Fields {
    pub fn validate(&self) -> Result<(), String> {
        for vector in [&self.inbox, &self.active] {
            if vector.entries.len() as u64 > u64::from(vector.capacity)
                || vector.entries.len() > i32::MAX as usize
            {
                return Err("window message vector extent differs".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Registered(u32),
    Dispatch(Message),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Registered(bool),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Next,
    Presence,
    AfterDispatch,
    Finish,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pass {
    stage: Stage,
    pending: Option<Request>,
    answer: Reply,
    slot: Option<usize>,
}
impl Default for Pass {
    fn default() -> Self {
        Self {
            stage: Stage::Entry,
            pending: None,
            answer: Reply::Done,
            slot: None,
        }
    }
}
impl Pass {
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (self.stage, self.pending) {
            (Stage::Presence, Some(Request::Registered(_)))
            | (Stage::AfterDispatch, Some(Request::Dispatch(_))) => self.slot.is_some(),
            (Stage::Presence, None) => {
                self.slot.is_some() && matches!(self.answer, Reply::Registered(_))
            }
            (_, None) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("window message pass lost its saved child".into())
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request)
            || !matches!(
                (request, reply),
                (Request::Registered(_), Reply::Registered(_))
                    | (Request::Dispatch(_), Reply::Done)
            )
        {
            return Err("window message reply differs from pending child".into());
        }
        self.pending = None;
        self.answer = reply;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        fields.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let request = loop {
            match self.stage {
                Stage::Entry => {
                    if fields.active.entries.is_empty() {
                        std::mem::swap(&mut fields.inbox, &mut fields.active);
                        fields.cursor = 0;
                    }
                    self.stage = Stage::Next;
                }
                Stage::Next => {
                    if fields.active.entries.is_empty() {
                        self.stage = Stage::Complete;
                        continue;
                    }
                    if fields.cursor as usize >= fields.active.entries.len() {
                        self.stage = Stage::Finish;
                        continue;
                    }
                    let slot = fields.cursor as usize;
                    let owner = fields.active.entries[slot].owner;
                    fields.cursor = fields.cursor.wrapping_add(1);
                    self.slot = Some(slot);
                    self.stage = Stage::Presence;
                    break Request::Registered(owner);
                }
                Stage::Presence => {
                    let Reply::Registered(present) = self.answer else {
                        unreachable!()
                    };
                    self.stage = Stage::AfterDispatch;
                    if present {
                        let message = *fields
                            .active
                            .entries
                            .get(self.slot.unwrap())
                            .ok_or("window message slot disappeared during membership child")?;
                        break Request::Dispatch(message);
                    }
                }
                Stage::AfterDispatch => {
                    self.slot = None;
                    self.stage = Stage::Next;
                }
                Stage::Finish => {
                    if fields.cursor as usize == fields.active.entries.len() {
                        fields.active.entries.clear();
                    }
                    self.stage = Stage::Complete;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(Some(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Deserialize)]
    struct Case {
        before: Fields,
        after: Fields,
        members: Vec<u32>,
        events: Vec<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct Oracle {
        cases: Vec<Case>,
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_message_pump_with_suspended_membership_and_dispatch()
    -> Result<(), Box<dyn std::error::Error>> {
        let oracle: Oracle = serde_json::from_str(include_str!(
            "../../../tests/fixtures/native-window-messages.json"
        ))?;
        assert_eq!(oracle.cases.len(), 129);
        for (index, case) in oracle.cases.into_iter().enumerate() {
            let mut fields = case.before;
            let mut pass = Pass::default();
            let mut events = Vec::new();
            while let Some(request) = pass.advance(&mut fields)? {
                let frozen = serde_json::to_vec(&(&pass, &fields))?;
                (pass, fields) = serde_json::from_slice(&frozen)?;
                assert_eq!(pass.advance(&mut fields)?, Some(request));
                assert_eq!(serde_json::to_vec(&(&pass, &fields))?, frozen);
                assert!(
                    pass.acknowledge(
                        request,
                        match request {
                            Request::Registered(_) => Reply::Done,
                            Request::Dispatch(_) => Reply::Registered(false),
                        }
                    )
                    .is_err()
                );
                assert_eq!(serde_json::to_vec(&(&pass, &fields))?, frozen);
                let reply = match request {
                    Request::Registered(owner) => {
                        let present = case.members.contains(&owner);
                        events.push(serde_json::json!({"kind":"registered","owner":owner,"present":present}));
                        Reply::Registered(present)
                    }
                    Request::Dispatch(message) => {
                        events.push(serde_json::json!({"kind":"dispatch","owner":message.owner,"words":message.words}));
                        Reply::Done
                    }
                };
                pass.acknowledge(request, reply)?;
            }
            assert!(pass.complete());
            assert_eq!(fields, case.after, "native case{index}");
            assert_eq!(events, case.events, "native events{index}");
        }
        Ok(())
    }
}
