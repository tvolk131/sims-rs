use super::{disjoint, unsupported};
use crate::native_world::{validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::Stop;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Kind {
    Bare,
    Cleared,
    Live,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct String {
    pub initial: Owner,
    pub owner: Owner,
    pub backing: Option<Owner>,
    allocation: Option<Owner>,
    pub content: Vec<u8>,
    pub kind: Kind,
}

pub(super) fn validate_allocation(owner: &Owner, bytes: usize) -> Result<(), Stop> {
    validate_owner(owner, bytes)?;
    if owner.token % 2 != 0 || bytes % 2 != 0 || owner.token + bytes as u64 > u32::MAX as u64 {
        return Err(unsupported(
            "wide backing lacks aligned native end pointers",
        ));
    }
    Ok(())
}

impl String {
    pub fn new(owner: Owner) -> Result<Self, Stop> {
        validate_owner(&owner, 12)?;
        Ok(Self {
            initial: owner.clone(),
            owner,
            backing: None,
            allocation: None,
            content: vec![],
            kind: Kind::Bare,
        })
    }

    pub fn clear(&mut self) {
        self.owner.data.fill(0);
        self.kind = Kind::Cleared;
    }

    pub fn initialized(&self, allocation: Owner, content: Vec<u8>) -> Result<Self, Stop> {
        validate_allocation(&allocation, allocation.data.len())?;
        if content.len() % 2 != 0 || content.len() + 2 > allocation.data.len() {
            return Err(unsupported(
                "wide string range lacks its two-byte terminator",
            ));
        }
        let mut result = self.clone();
        let mut backing = allocation.clone();
        backing.data[..content.len()].copy_from_slice(&content);
        backing.data[content.len()..content.len() + 2].fill(0);
        write_word(&mut result.owner, 0, allocation.token as u32);
        write_word(
            &mut result.owner,
            4,
            (allocation.token + content.len() as u64) as u32,
        );
        write_word(
            &mut result.owner,
            8,
            (allocation.token + allocation.data.len() as u64) as u32,
        );
        result.backing = Some(backing);
        result.allocation = Some(allocation);
        result.content = content;
        result.kind = Kind::Live;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        validate_owner(&self.initial, 12)?;
        validate_owner(&self.owner, 12)?;
        if self.owner.token != self.initial.token || self.content.len() % 2 != 0 {
            return Err(unsupported("wide descriptor lost its incoming identity"));
        }
        let mut expected = self.initial.clone();
        match self.kind {
            Kind::Bare | Kind::Cleared => {
                if self.backing.is_some() || self.allocation.is_some() || !self.content.is_empty() {
                    return Err(unsupported("unconstructed wide descriptor owns a backing"));
                }
                if self.kind == Kind::Cleared {
                    expected.data.fill(0);
                }
            }
            Kind::Live | Kind::Released => {
                let allocation = self
                    .allocation
                    .as_ref()
                    .ok_or_else(|| unsupported("wide allocation metadata absent"))?;
                let backing = self
                    .backing
                    .as_ref()
                    .ok_or_else(|| unsupported("wide backing absent"))?;
                validate_allocation(allocation, allocation.data.len())?;
                validate_allocation(backing, allocation.data.len())?;
                if backing.token != allocation.token || self.content.len() + 2 > backing.data.len()
                {
                    return Err(unsupported("wide string exceeds its retained allocation"));
                }
                let mut expected_backing = allocation.clone();
                expected_backing.data[..self.content.len()].copy_from_slice(&self.content);
                expected_backing.data[self.content.len()..self.content.len() + 2].fill(0);
                write_word(&mut expected, 0, backing.token as u32);
                write_word(
                    &mut expected,
                    4,
                    (backing.token + self.content.len() as u64) as u32,
                );
                write_word(
                    &mut expected,
                    8,
                    (backing.token + backing.data.len() as u64) as u32,
                );
                if *backing != expected_backing {
                    return Err(unsupported(
                        "wide construction changed untouched backing bytes",
                    ));
                }
                disjoint(vec![
                    (self.owner.token, 12),
                    (backing.token, backing.data.len()),
                ])?;
            }
        }
        if expected != self.owner {
            return Err(unsupported(
                "wide descriptor differs from its exact phase writes",
            ));
        }
        Ok(())
    }

    pub fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        let mut spans = if self.kind == Kind::Released {
            vec![]
        } else {
            vec![(self.owner.token, 12)]
        };
        if self.kind == Kind::Live {
            let backing = self.backing.as_ref().unwrap();
            spans.push((backing.token, backing.data.len()));
        }
        Ok(spans)
    }
}
