use super::{Host, Name};
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Prefix,
    Append,
    Allocate,
    Accept,
    Release,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Assignment {
    input: Vec<u8>,
    phase: Phase,
    requested: u32,
    reply: Option<Allocated>,
    pending: Option<Name>,
}

impl Assignment {
    pub(super) fn validate(&self, name: &Name) -> Result<(), Stop> {
        name.validate()?;
        if self.input.len() > 511
            || self.input.contains(&0)
            || (self.phase == Phase::Accept) != self.reply.is_some()
            || (self.phase == Phase::Release) != self.pending.is_some()
        {
            return Err(Stop::Unsupported(
                "matrix name cursor and retained replies differ".into(),
            ));
        }
        if matches!(
            self.phase,
            Phase::Append | Phase::Allocate | Phase::Accept | Phase::Release
        ) && self.input.len() <= name.length
        {
            return Err(Stop::Unsupported(
                "matrix name append has no remaining input".into(),
            ));
        }
        if matches!(self.phase, Phase::Allocate | Phase::Accept | Phase::Release) {
            let requested = name
                .length
                .checked_add(name.length.max(self.input.len() - name.length))
                .and_then(|capacity| capacity.checked_add(1));
            if self.input.len() < name.data.len() || requested != Some(self.requested as usize) {
                return Err(Stop::Unsupported(
                    "matrix name allocation request differs".into(),
                ));
            }
        }
        if let Some(pending) = &self.pending {
            pending.validate()?;
            if pending.token == name.token
                || pending.data.len() != self.requested as usize
                || pending.bytes()? != self.input
            {
                return Err(Stop::Unsupported(
                    "pending matrix name differs from its assignment".into(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn input(&self) -> &[u8] {
        &self.input
    }

    pub(super) fn retained_spans(&self) -> Vec<(u64, usize)> {
        let mut spans = Vec::new();
        if let Some(Allocated::Bytes { token, data }) = &self.reply {
            spans.push((*token, data.len()));
        }
        if let Some(name) = &self.pending {
            spans.push((name.token, name.data.len()));
        }
        spans
    }

    pub(super) fn new(input: Vec<u8>) -> Self {
        Self {
            input,
            phase: Phase::Prefix,
            requested: 0,
            reply: None,
            pending: None,
        }
    }

    pub(super) fn step(&mut self, name: &mut Name, host: &mut impl Host) -> Result<bool, Stop> {
        self.validate(name)?;
        match self.phase {
            Phase::Prefix => {
                let copied = self.input.len().min(name.length);
                name.data[..copied].copy_from_slice(&self.input[..copied]);
                if self.input.len() <= name.length {
                    name.data[self.input.len()] = 0;
                    name.length = self.input.len();
                    self.phase = Phase::Complete;
                } else {
                    self.phase = Phase::Append;
                }
            }
            Phase::Append => {
                if self.input.len() < name.data.len() {
                    name.data[name.length..self.input.len()]
                        .copy_from_slice(&self.input[name.length..]);
                    name.length = self.input.len();
                    name.data[name.length] = 0;
                    self.phase = Phase::Complete;
                } else {
                    let appended = self.input.len() - name.length;
                    let capacity = name
                        .length
                        .checked_add(name.length.max(appended))
                        .and_then(|capacity| capacity.checked_add(1))
                        .ok_or_else(|| {
                            Stop::Unsupported("matrix name capacity arithmetic overflow".into())
                        })?;
                    self.requested = u32::try_from(capacity).map_err(|_| {
                        Stop::Unsupported("matrix name allocation exceeds native width".into())
                    })?;
                    self.phase = Phase::Allocate;
                }
            }
            Phase::Allocate => {
                self.reply = Some(host.allocate_name(self.requested)?);
                self.phase = Phase::Accept;
            }
            Phase::Accept => {
                let (token, data) = match self.reply.as_ref() {
                    Some(Allocated::Bytes { token, data })
                        if *token != 0
                            && *token != name.token
                            && data.len() == self.requested as usize =>
                    {
                        (*token, data)
                    }
                    Some(Allocated::Null) => {
                        return Err(Stop::Unavailable(
                            "matrix name allocation has no writable returned buffer".into(),
                        ));
                    }
                    _ => {
                        return Err(Stop::Unsupported(
                            "invalid retained matrix name allocator reply".into(),
                        ));
                    }
                };
                let mut data = data.clone();
                data[..name.length].copy_from_slice(&name.data[..name.length]);
                data[name.length..self.input.len()].copy_from_slice(&self.input[name.length..]);
                data[self.input.len()] = 0;
                self.pending = Some(Name {
                    token,
                    length: self.input.len(),
                    data,
                });
                self.reply = None;
                self.phase = Phase::Release;
            }
            Phase::Release => {
                let pending = self.pending.as_ref().ok_or_else(|| {
                    Stop::Unavailable("pending matrix name allocation absent".into())
                })?;
                pending.validate()?;
                host.release_name(name.token)?;
                *name = self.pending.take().unwrap();
                self.phase = Phase::Complete;
            }
            Phase::Complete => {}
        }
        Ok(self.phase == Phase::Complete)
    }
}
