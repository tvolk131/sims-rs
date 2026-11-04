//! Source-bound framework timer/message owners before C8 drawing. Cursor
//! sampling, OS publication, time and live widget callbacks stay explicit.
use super::completed_lot::Memory;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::ui_effect::widgets::{
    messages,
    timers::{self, dispatch},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Timers,
    Messages,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Timer(dispatch::Request),
    Message(messages::Request),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Timer(dispatch::Reply),
    Message(messages::Reply),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    phase: Phase,
    registry: timers::Registry,
    timer_capacity: u32,
    flags: dispatch::Flags,
    members: Vec<u32>,
    special: u32,
    messages: messages::Fields,
    timer_pass: dispatch::Pass,
    message_pass: messages::Pass,
}
fn word(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().unwrap())
}
pub fn import(source: &[u8]) -> Result<State> {
    ensure!(
        sims_fixtures::digest(source)
            == "fb38fab9cd8f1d3d9a66e3b8f55ed48135f08d842a0d2b8a636175eaa5bdb00b",
        "framework window source differs"
    );
    let v: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        v["tick"] == 89073 && v["executes_target_code"] == false && v["native_parity_credit"] == 0,
        "framework window source epoch differs"
    );
    let memory = Memory::new(&v)?;
    let header = memory.read(0x6fedf8, 0x240)?;
    let vector = |offset, stride| -> Result<(Vec<Vec<u32>>, u32)> {
        let (begin, end, capacity) = (
            word(&header, offset),
            word(&header, offset + 4),
            word(&header, offset + 8),
        );
        ensure!(
            begin <= end
                && end <= capacity
                && (end - begin) % stride == 0
                && (capacity - begin) % stride == 0
                && capacity - begin <= 65536,
            "framework vector extent differs"
        );
        let raw = if begin == end {
            Vec::new()
        } else {
            memory.read(begin, (end - begin).try_into()?)?
        };
        let rows = raw
            .chunks_exact(stride as usize)
            .map(|r| {
                r.chunks_exact(4)
                    .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
                    .collect()
            })
            .collect();
        Ok((rows, (capacity - begin) / stride))
    };
    let (owners, _) = vector(0x34, 4)?;
    let members = owners.into_iter().map(|v| v[0]).collect();
    let (rows, timer_capacity) = vector(0x1cc, 16)?;
    let registry = timers::Registry {
        entries: rows
            .into_iter()
            .map(|w| timers::Entry {
                owner: w[0],
                interval: w[1],
                deadline: w[2],
                argument: w[3],
            })
            .collect(),
        first: word(&header, 0x1d8) as i32,
        second: word(&header, 0x1dc) as i32,
    };
    let mut queues = Vec::new();
    for offset in [0x154, 0x160] {
        let (rows, capacity) = vector(offset, 20)?;
        queues.push(messages::Vector {
            entries: rows
                .into_iter()
                .map(|w| messages::Message {
                    owner: w[0],
                    words: w[1..].try_into().unwrap(),
                })
                .collect(),
            capacity,
        });
    }
    let mut queues = queues.into_iter();
    let state = State {
        phase: Phase::Timers,
        registry,
        timer_capacity,
        flags: dispatch::Flags {
            running: header[0x1e0],
            pointer_pending: header[0x1e1],
        },
        members,
        special: word(&header, 0x1c0),
        messages: messages::Fields {
            inbox: queues.next().unwrap(),
            active: queues.next().unwrap(),
            cursor: word(&header, 0x16c),
        },
        timer_pass: dispatch::Pass::default(),
        message_pass: messages::Pass::default(),
    };
    state.validate()?;
    Ok(state)
}
impl State {
    pub fn at_entry(&self) -> bool {
        self.phase == Phase::Timers
            && self.timer_pass == dispatch::Pass::default()
            && self.message_pass == messages::Pass::default()
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn timers(&self) -> (&timers::Registry, dispatch::Flags) {
        (&self.registry, self.flags)
    }
    pub fn messages(&self) -> &messages::Fields {
        &self.messages
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.registry.entries.len() as u64 <= u64::from(self.timer_capacity),
            "framework timer capacity differs"
        );
        self.timer_pass.validate().map_err(anyhow::Error::msg)?;
        self.message_pass.validate().map_err(anyhow::Error::msg)?;
        self.messages.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            match self.phase {
                Phase::Timers => !self.timer_pass.complete() && !self.message_pass.complete(),
                Phase::Messages => self.timer_pass.complete() && !self.message_pass.complete(),
                Phase::Complete => self.timer_pass.complete() && self.message_pass.complete(),
            },
            "framework pass completion differs"
        );
        Ok(())
    }
    pub fn advance(&mut self) -> Result<Option<Request>> {
        self.validate()?;
        for _ in 0..4096 {
            match self.phase {
                Phase::Timers => match self
                    .timer_pass
                    .advance(&mut self.registry, &mut self.flags)
                    .map_err(anyhow::Error::msg)?
                {
                    Some(request @ dispatch::Request::Registered(owner)) => {
                        let present = timers::registered(owner, self.special, &self.members);
                        self.timer_pass
                            .acknowledge(request, dispatch::Reply::Registered(present))
                            .map_err(anyhow::Error::msg)?;
                    }
                    Some(request) => return Ok(Some(Request::Timer(request))),
                    None => self.phase = Phase::Messages,
                },
                Phase::Messages => match self
                    .message_pass
                    .advance(&mut self.messages)
                    .map_err(anyhow::Error::msg)?
                {
                    Some(request @ messages::Request::Registered(owner)) => {
                        let present = timers::registered(owner, self.special, &self.members);
                        self.message_pass
                            .acknowledge(request, messages::Reply::Registered(present))
                            .map_err(anyhow::Error::msg)?;
                    }
                    Some(request) => return Ok(Some(Request::Message(request))),
                    None => self.phase = Phase::Complete,
                },
                Phase::Complete => {
                    self.validate()?;
                    return Ok(None);
                }
            }
        }
        anyhow::bail!("framework pass exceeded work budget")
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<()> {
        self.validate()?;
        match (self.phase, request, reply) {
            (Phase::Timers, Request::Timer(request), Reply::Timer(reply)) => {
                ensure!(
                    !matches!(request, dispatch::Request::Registered(_)),
                    "framework owns membership replies"
                );
                self.timer_pass
                    .acknowledge(request, reply)
                    .map_err(anyhow::Error::msg)
            }
            (Phase::Messages, Request::Message(request), Reply::Message(reply)) => {
                ensure!(
                    !matches!(request, messages::Request::Registered(_)),
                    "framework owns membership replies"
                );
                self.message_pass
                    .acknowledge(request, reply)
                    .map_err(anyhow::Error::msg)
            }
            _ => anyhow::bail!("framework reply differs from active pass"),
        }
    }
}
