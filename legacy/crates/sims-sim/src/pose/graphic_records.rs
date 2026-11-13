//! Base Animator record walks: clear 584b35, destroy 584bee, bind 584c47.
//! Indices advance through the live vectors after each service. Bind captures
//! one skin before traversal; destruction clears the live slot after release.
use super::graphic_appearance::Model;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vectors {
    pub generic: Vec<usize>,
    pub accessories: Vec<Option<usize>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    Clear,
    DestroyAccessories,
    BindAccessories,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Clear(usize),
    /// 5834ff followed by deallocation of this captured record identity.
    Destroy(usize),
    /// 583559 with pointers to the record's live name and texture strings.
    Bind {
        record: usize,
        skin: u64,
    },
    /// Increment the captured skin's +ac and +b4 and set +98=1. These are
    /// inline stores, so the host must not introduce an external callback.
    Invalidate {
        skin: u64,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Generic,
    Accessories,
    Invalidate,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    entry: Entry,
    stage: Stage,
    index: usize,
    skin: Option<u64>,
    pending: Option<Request>,
}
impl Update {
    pub fn new(entry: Entry) -> Self {
        Self {
            entry,
            stage: Stage::Start,
            index: 0,
            skin: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.skin == Some(0)
            || self.entry == Entry::BindAccessories
                && self.stage != Stage::Start
                && self.skin.is_none()
            || self
                .pending
                .as_ref()
                .is_some_and(|r| match (self.entry, self.stage, r) {
                    (Entry::Clear, Stage::Generic | Stage::Accessories, Request::Clear(_)) => false,
                    (Entry::DestroyAccessories, Stage::Accessories, Request::Destroy(_)) => false,
                    (Entry::BindAccessories, Stage::Accessories, Request::Bind { skin, .. })
                    | (Entry::BindAccessories, Stage::Invalidate, Request::Invalidate { skin }) => {
                        Some(*skin) != self.skin
                    }
                    _ => true,
                })
        {
            return Err("invalid base Animator record walk".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        vectors: &mut Vectors,
        model: Option<Model>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    self.stage = if self.entry == Entry::Clear {
                        Stage::Generic
                    } else {
                        if self.entry == Entry::BindAccessories {
                            self.skin = Some(
                                model
                                    .filter(|m| m.model != 0 && m.skin != 0)
                                    .ok_or("accessory binding model absent")?
                                    .skin,
                            );
                        }
                        Stage::Accessories
                    };
                    continue;
                }
                Stage::Generic => {
                    if let Some(&id) = vectors.generic.get(self.index) {
                        Request::Clear(id)
                    } else {
                        self.index = 0;
                        self.stage = Stage::Accessories;
                        continue;
                    }
                }
                Stage::Accessories => {
                    if let Some(&record) = vectors.accessories.get(self.index) {
                        match (self.entry, record) {
                            (Entry::Clear, Some(id)) => Request::Clear(id),
                            (Entry::DestroyAccessories, Some(id)) => Request::Destroy(id),
                            (Entry::DestroyAccessories, None) => {
                                self.index += 1;
                                continue;
                            }
                            (Entry::BindAccessories, Some(id)) => Request::Bind {
                                record: id,
                                skin: self.skin.unwrap(),
                            },
                            _ => return Err("null accessory in native clear/bind walk".into()),
                        }
                    } else {
                        self.stage = if self.entry == Entry::BindAccessories {
                            Stage::Invalidate
                        } else {
                            if self.entry == Entry::DestroyAccessories {
                                vectors.accessories.clear();
                            }
                            Stage::Done
                        };
                        continue;
                    }
                }
                Stage::Invalidate => Request::Invalidate {
                    skin: self.skin.unwrap(),
                },
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            self.validate()?;
            return Ok(self.pending.clone());
        }
    }
    pub fn acknowledge(&mut self, vectors: &mut Vectors, request: &Request) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("stale base Animator record acknowledgment".into());
        }
        if matches!(request, Request::Destroy(_)) {
            // The native code reloads the vector base, then nulls index i.
            // A callback may have changed which identity occupies that slot.
            if let Some(slot) = vectors.accessories.get_mut(self.index) {
                *slot = None;
            }
        }
        if matches!(request, Request::Invalidate { .. }) {
            self.stage = Stage::Done;
        } else {
            self.index = self
                .index
                .checked_add(1)
                .ok_or("base Animator record index overflow")?;
        }
        self.pending = None;
        Ok(())
    }
}
