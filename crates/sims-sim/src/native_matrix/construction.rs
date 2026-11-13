use super::{Host, Id, Name, Naming, Node, Registry};
use crate::saved_matrix::{Allocated, Allocation, Matrix, Resize, Stop};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Owner,
    ReserveName,
    AcceptName,
    Header,
    Naming,
    Resize,
    Clear,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    owner: Id,
    stride: i32,
    phase: Phase,
    reply: Option<Allocated>,
    name: Option<Name>,
    naming: Naming,
    resize: Resize,
}

impl Construction {
    pub(crate) fn validate_for(
        &self,
        registry: &Registry,
        owner: Id,
        stride: i32,
        width: i32,
        height: i32,
        input: &[u8],
    ) -> Result<(), Stop> {
        if self.owner != owner
            || self.stride != stride
            || self.resize.dimensions() != (width, height)
            || self.naming.input != input
        {
            return Err(Stop::Unsupported(
                "matrix constructor lost its original profile".into(),
            ));
        }
        self.validate(registry)
    }

    fn validate(&self, registry: &Registry) -> Result<(), Stop> {
        registry.validate()?;
        let (width, height) = self.resize.dimensions();
        if self.owner == 0
            || width != height
            || !((width == 64 && matches!(self.stride, 1 | 2 | 4 | 14))
                || (width == 128 && self.stride == 2))
            || (self.phase == Phase::AcceptName) != self.reply.is_some()
            || (self.phase == Phase::Header) != self.name.is_some()
        {
            return Err(Stop::Unsupported(
                "matrix constructor cursor and retained replies differ".into(),
            ));
        }
        if matches!(
            self.phase,
            Phase::Owner | Phase::ReserveName | Phase::AcceptName | Phase::Header
        ) {
            if registry.nodes.contains_key(&self.owner)
                || self.naming != Naming::external(self.owner, self.naming.input.clone())?
                || self.resize != Resize::new(width, height)
            {
                return Err(Stop::Unsupported(
                    "matrix constructor published ownership before its header".into(),
                ));
            }
            if let Some(name) = &self.name {
                name.validate()?;
            }
            return Ok(());
        }
        let node = registry
            .nodes
            .get(&self.owner)
            .ok_or_else(|| Stop::Unavailable("constructed matrix owner absent".into()))?;
        if node.matrix.stride != self.stride {
            return Err(Stop::Unsupported(
                "constructed matrix stride changed".into(),
            ));
        }
        self.naming
            .validate_for(registry, self.owner, &self.naming.input)?;
        if self.phase == Phase::Naming {
            if self.resize != Resize::new(width, height)
                || node.matrix.width != -1
                || node.matrix.height != -1
                || node.matrix.allocation != Allocation::Null
            {
                return Err(Stop::Unsupported(
                    "matrix naming lost its fresh unresized owner".into(),
                ));
            }
        } else {
            if self.naming.phase != super::NamingPhase::Complete {
                return Err(Stop::Unsupported(
                    "matrix resize precedes completed naming".into(),
                ));
            }
            self.resize.validate_fresh(&node.matrix)?;
            if matches!(self.phase, Phase::Clear | Phase::Complete) && !self.resize.is_complete() {
                return Err(Stop::Unsupported(
                    "matrix clear precedes completed resize".into(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn retained_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        let mut spans = self.naming.retained_spans();
        if let Some(Allocated::Bytes { token, data }) = &self.reply {
            spans.push((*token, data.len()));
        }
        if let Some(name) = &self.name {
            spans.push((name.token, name.data.len()));
        }
        if let Some(span) = self.resize.pending_span(self.stride)? {
            spans.push(span);
        }
        Ok(spans)
    }

    pub fn new(
        owner: Id,
        stride: i32,
        width: i32,
        height: i32,
        name: Vec<u8>,
    ) -> Result<Self, Stop> {
        if owner == 0
            || width != height
            || !((width == 64 && matches!(stride, 1 | 2 | 4 | 14)) || (width == 128 && stride == 2))
        {
            return Err(Stop::Unsupported(
                "outside original world named-matrix constructor profile".into(),
            ));
        }
        Ok(Self {
            owner,
            stride,
            phase: Phase::Owner,
            reply: None,
            name: None,
            naming: Naming::external(owner, name)?,
            resize: Resize::new(width, height),
        })
    }

    pub fn step(
        &mut self,
        registry: &mut Registry,
        host: &mut impl Host,
    ) -> Result<Option<Id>, Stop> {
        self.validate(registry)?;
        match self.phase {
            Phase::Owner => {
                if registry.nodes.contains_key(&self.owner) {
                    return Err(Stop::Unsupported(
                        "named matrix constructor owner already exists".into(),
                    ));
                }
                host.require_owner(self.owner, 32)?;
                self.phase = Phase::ReserveName;
            }
            Phase::ReserveName => {
                self.reply = Some(host.allocate_name(8)?);
                self.phase = Phase::AcceptName;
            }
            Phase::AcceptName => {
                match self.reply.as_ref() {
                    Some(Allocated::Bytes { token, data }) if *token != 0 && data.len() == 8 => {
                        let mut data = data.clone();
                        data[0] = 0;
                        self.name = Some(Name {
                            token: *token,
                            length: 0,
                            data,
                        });
                    }
                    Some(Allocated::Null) => {
                        return Err(Stop::Unavailable(
                            "matrix constructor name has no writable returned allocation".into(),
                        ));
                    }
                    _ => {
                        return Err(Stop::Unsupported(
                            "invalid initial matrix name allocation reply".into(),
                        ));
                    }
                }
                self.reply = None;
                self.phase = Phase::Header;
            }
            Phase::Header => {
                if registry.nodes.contains_key(&self.owner) {
                    return Err(Stop::Unsupported(
                        "matrix owner appeared before header construction".into(),
                    ));
                }
                let name = self.name.as_ref().ok_or_else(|| {
                    Stop::Unavailable("constructed matrix name allocation absent".into())
                })?;
                if name.token == self.owner || registry.allocation_in_use(name.token) {
                    return Err(Stop::Unsupported(
                        "constructed matrix name allocation aliases a live owner".into(),
                    ));
                }
                let name = self.name.take().ok_or_else(|| {
                    Stop::Unavailable("constructed matrix name allocation absent".into())
                })?;
                registry.nodes.insert(
                    self.owner,
                    Node {
                        next: registry.head,
                        name,
                        matrix: Matrix {
                            width: -1,
                            height: -1,
                            stride: self.stride,
                            allocation: Allocation::Null,
                        },
                    },
                );
                registry.head = Some(self.owner);
                self.phase = Phase::Naming;
            }
            Phase::Naming => {
                if self.naming.step(registry, host)? {
                    self.phase = Phase::Resize;
                }
            }
            Phase::Resize => {
                if self
                    .resize
                    .step(&mut registry.node_mut(self.owner)?.matrix, host)?
                    .is_some()
                {
                    self.phase = Phase::Clear;
                }
            }
            Phase::Clear => {
                let matrix = &mut registry.node_mut(self.owner)?.matrix;
                let count = (matrix.width as usize)
                    .checked_mul(matrix.height as usize)
                    .and_then(|count| count.checked_mul(matrix.stride as usize))
                    .ok_or_else(|| {
                        Stop::Unsupported("matrix constructor clear extent overflow".into())
                    })?;
                let Allocation::Known(backing) = &mut matrix.allocation else {
                    return Err(Stop::Unavailable(
                        "native matrix clear has no returned writable backing".into(),
                    ));
                };
                if backing.data.len() < count {
                    return Err(Stop::Unsupported(
                        "matrix constructor clear exceeds retained allocation".into(),
                    ));
                }
                backing.data[..count].fill(0);
                self.phase = Phase::Complete;
            }
            Phase::Complete => {}
        }
        Ok((self.phase == Phase::Complete).then_some(self.owner))
    }
}
