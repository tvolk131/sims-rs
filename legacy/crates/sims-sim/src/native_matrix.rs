use crate::saved_matrix::{self, Allocated, Matrix, Stop};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

mod construction;
mod name;
pub use construction::Construction;
use name::Assignment;

pub type Id = u64;

pub trait Host: saved_matrix::Host {
    fn require_owner(&self, owner: Id, bytes: u32) -> Result<(), Stop>;
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop>;
    fn release_name(&mut self, token: u64) -> Result<(), Stop>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Name {
    pub token: u64,
    pub length: usize,
    pub data: Vec<u8>,
}

impl Name {
    pub fn validate(&self) -> Result<(), Stop> {
        if self.token == 0
            || self.length >= self.data.len()
            || self.data.len() > saved_matrix::MAX_ALLOCATION_BYTES
            || self.data[self.length] != 0
        {
            return Err(Stop::Unsupported(
                "invalid retained matrix name allocation".into(),
            ));
        }
        Ok(())
    }

    pub fn bytes(&self) -> Result<&[u8], Stop> {
        self.validate()?;
        Ok(&self.data[..self.length])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub next: Option<Id>,
    pub name: Name,
    pub matrix: Matrix,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub head: Option<Id>,
    pub nodes: BTreeMap<Id, Node>,
}

impl Registry {
    pub fn validate(&self) -> Result<(), Stop> {
        if self.nodes.contains_key(&0) {
            return Err(Stop::Unsupported("null named matrix identity".into()));
        }
        let mut allocations: BTreeSet<_> = self.nodes.keys().copied().collect();
        for node in self.nodes.values() {
            node.name.validate()?;
            if !allocations.insert(node.name.token) {
                return Err(Stop::Unsupported(
                    "named matrix allocation identities alias".into(),
                ));
            }
            if let saved_matrix::Allocation::Known(backing) = &node.matrix.allocation {
                if backing.token == 0 || !allocations.insert(backing.token) {
                    return Err(Stop::Unsupported(
                        "named matrix backing identities alias or are null".into(),
                    ));
                }
            }
        }
        let mut following = self.head;
        let mut seen = BTreeSet::new();
        while let Some(owner) = following {
            if !seen.insert(owner) {
                return Err(Stop::Unsupported("cyclic named matrix registry".into()));
            }
            following = self
                .nodes
                .get(&owner)
                .ok_or_else(|| Stop::Unavailable("named matrix link owner absent".into()))?
                .next;
        }
        Ok(())
    }

    fn node_mut(&mut self, owner: Id) -> Result<&mut Node, Stop> {
        self.nodes
            .get_mut(&owner)
            .ok_or_else(|| Stop::Unavailable("named matrix owner absent".into()))
    }

    fn allocation_in_use(&self, token: u64) -> bool {
        self.nodes.contains_key(&token)
            || self.nodes.values().any(|node| {
                node.name.token == token
                    || matches!(&node.matrix.allocation,
                saved_matrix::Allocation::Known(backing) if backing.token == token)
            })
    }

    fn unlink(&mut self, owner: Id) -> Result<(), Stop> {
        self.validate()?;
        if !self.nodes.contains_key(&owner) {
            return Err(Stop::Unavailable("named matrix owner absent".into()));
        }
        let mut previous = None;
        let mut following = self.head;
        while let Some(current) = following {
            let next = self.nodes[&current].next;
            if current == owner {
                if let Some(previous) = previous {
                    self.node_mut(previous)?.next = next;
                } else {
                    self.head = next;
                }
                break;
            }
            previous = Some(current);
            following = next;
        }
        Ok(())
    }

    pub fn find(&self, name: &[u8]) -> Result<Option<Id>, Stop> {
        self.validate()?;
        let mut following = self.head;
        while let Some(owner) = following {
            let node = &self.nodes[&owner];
            if node.name.bytes()? == name {
                return Ok(Some(owner));
            }
            following = node.next;
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NamingPhase {
    Unlink,
    Assign,
    Find,
    Publish,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Naming {
    owner: Id,
    input: Vec<u8>,
    suffix: u32,
    candidate: bool,
    phase: NamingPhase,
    assignment: Option<Assignment>,
}

impl Naming {
    pub(crate) fn validate_for(
        &self,
        registry: &Registry,
        owner: Id,
        input: &[u8],
    ) -> Result<(), Stop> {
        if self.owner != owner || self.input != input {
            return Err(Stop::Unsupported(
                "matrix naming changed its retained owner or input".into(),
            ));
        }
        self.validate(registry)
    }

    fn validate(&self, registry: &Registry) -> Result<(), Stop> {
        registry.validate()?;
        if self.owner == 0
            || self.input.len() > 511
            || self.input.contains(&0)
            || matches!(self.phase, NamingPhase::Unlink | NamingPhase::Assign)
                != self.assignment.is_some()
        {
            return Err(Stop::Unsupported("invalid matrix naming cursor".into()));
        }
        let node = registry
            .nodes
            .get(&self.owner)
            .ok_or_else(|| Stop::Unavailable("matrix naming owner absent".into()))?;
        if self.phase == NamingPhase::Unlink
            && self != &Self::external(self.owner, self.input.clone())?
        {
            return Err(Stop::Unsupported(
                "matrix naming rewound an active assignment".into(),
            ));
        }
        if let Some(assignment) = &self.assignment {
            assignment.validate(&node.name)?;
            let mut expected = self.input.clone();
            if self.candidate {
                if self.suffix > i32::MAX as u32 {
                    return Err(Stop::Unsupported(
                        "matrix name suffix outside signed native profile".into(),
                    ));
                }
                expected.extend_from_slice(self.suffix.to_string().as_bytes());
            } else if self.suffix != 0 {
                return Err(Stop::Unsupported(
                    "matrix naming suffix has no candidate".into(),
                ));
            }
            if assignment.input() != expected {
                return Err(Stop::Unsupported(
                    "matrix name assignment lost its candidate".into(),
                ));
            }
        }
        let mut following = registry.head;
        let mut linked = false;
        while let Some(identity) = following {
            linked |= identity == self.owner;
            following = registry.nodes[&identity].next;
        }
        if (self.phase == NamingPhase::Complete && !linked)
            || (matches!(
                self.phase,
                NamingPhase::Assign | NamingPhase::Find | NamingPhase::Publish
            ) && linked)
        {
            return Err(Stop::Unsupported(
                "matrix naming publication and cursor differ".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn retained_spans(&self) -> Vec<(u64, usize)> {
        self.assignment
            .as_ref()
            .map_or_else(Vec::new, Assignment::retained_spans)
    }

    pub fn external(owner: Id, input: Vec<u8>) -> Result<Self, Stop> {
        if owner == 0 || input.len() > 511 || input.contains(&0) {
            return Err(Stop::Unsupported(
                "matrix naming exceeds the external C-string profile".into(),
            ));
        }
        Ok(Self {
            owner,
            assignment: Some(Assignment::new(input.clone())),
            input,
            suffix: 0,
            candidate: false,
            phase: NamingPhase::Unlink,
        })
    }

    pub fn step(&mut self, registry: &mut Registry, host: &mut impl Host) -> Result<bool, Stop> {
        self.validate(registry)?;
        match self.phase {
            NamingPhase::Unlink => {
                registry.unlink(self.owner)?;
                self.phase = NamingPhase::Assign;
            }
            NamingPhase::Assign => {
                let name = &mut registry.node_mut(self.owner)?.name;
                if self
                    .assignment
                    .as_mut()
                    .ok_or_else(|| {
                        Stop::Unavailable("matrix name assignment cursor absent".into())
                    })?
                    .step(name, host)?
                {
                    self.assignment = None;
                    if self.candidate {
                        self.suffix = self.suffix.checked_add(1).ok_or_else(|| {
                            Stop::Unsupported("matrix name suffix overflow".into())
                        })?;
                    }
                    self.phase = NamingPhase::Find;
                }
            }
            NamingPhase::Find => {
                let name = registry
                    .nodes
                    .get(&self.owner)
                    .ok_or_else(|| Stop::Unavailable("matrix naming owner absent".into()))?
                    .name
                    .bytes()?;
                if !name.is_empty() && registry.find(name)?.is_none() {
                    self.phase = NamingPhase::Publish;
                } else {
                    if self.suffix > i32::MAX as u32 {
                        return Err(Stop::Unsupported(
                            "matrix name suffix outside signed native profile".into(),
                        ));
                    }
                    let mut candidate = self.input.clone();
                    candidate.extend_from_slice(self.suffix.to_string().as_bytes());
                    if candidate.len() > 511 {
                        return Err(Stop::Unsupported(
                            "matrix naming exceeds native stack buffer".into(),
                        ));
                    }
                    self.assignment = Some(Assignment::new(candidate));
                    self.candidate = true;
                    self.phase = NamingPhase::Assign;
                }
            }
            NamingPhase::Publish => {
                let head = registry.head;
                registry.node_mut(self.owner)?.next = head;
                registry.head = Some(self.owner);
                self.phase = NamingPhase::Complete;
            }
            NamingPhase::Complete => {}
        }
        Ok(self.phase == NamingPhase::Complete)
    }
}

#[cfg(test)]
mod tests;
