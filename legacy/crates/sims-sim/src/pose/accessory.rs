//! Native 58d88f/58d919 additional appearance records. Replacement removes
//! every exact-name match before appending a new, initially empty record.
use super::appearance::{Record, Records};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Clear(usize),
    /// The destructor clears again before the record is freed and erased.
    Destroy(usize),
    Bind {
        record: usize,
        name: String,
        texture: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Search,
    Destroy(usize),
    Append,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    name: String,
    texture: Option<String>,
    stage: Stage,
    pending: Option<Request>,
}
impl Execution {
    pub fn new(name: String, texture: Option<String>) -> Self {
        Self {
            name,
            texture,
            stage: Stage::Search,
            pending: None,
        }
    }
    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Done && self.pending.is_none()
    }
    pub fn pending_request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn matches(&self, name: &str, texture: Option<&str>) -> bool {
        self.name == name && self.texture.as_deref() == texture
    }
    pub fn validate(&self, records: &Records) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for &id in &records.accessories {
            if id >= records.entries.len()
                || !ids.insert(id)
                || [
                    records.generic,
                    records.head,
                    records.hands[0],
                    records.hands[1],
                    records.body[0],
                    records.body[1],
                ]
                .contains(&id)
            {
                return Err("invalid accessory record identity".into());
            }
        }
        let valid = match (&self.stage, &self.pending) {
            (Stage::Search, Some(Request::Clear(id))) => {
                records
                    .accessories
                    .iter()
                    .find(|&&r| records.entries[r].name == self.name)
                    == Some(id)
            }
            (Stage::Destroy(id), None) | (Stage::Destroy(id), Some(Request::Destroy(_))) => {
                ids.contains(id)
                    && records.entries[*id].name == self.name
                    && records.entries[*id].binding.is_none()
                    && self
                        .pending
                        .as_ref()
                        .is_none_or(|r| r == &Request::Destroy(*id))
            }
            (
                Stage::Append,
                Some(Request::Bind {
                    record,
                    name,
                    texture,
                }),
            ) => {
                ids.contains(record)
                    && *record + 1 == records.entries.len()
                    && records.entries[*record] == Record::default()
                    && *name == self.name
                    && self.texture.as_ref() == Some(texture)
            }
            (Stage::Append, None) => self.texture.is_some(),
            (_, None) => true,
            _ => false,
        };
        if !valid {
            return Err("invalid accessory continuation".into());
        }
        Ok(())
    }
    pub fn next(&mut self, records: &mut Records) -> Result<Option<Request>, String> {
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let request = match self.stage {
                Stage::Search => {
                    let mut found = None;
                    for &id in &records.accessories {
                        if records
                            .entries
                            .get(id)
                            .ok_or("accessory record absent")?
                            .name
                            == self.name
                        {
                            found = Some(id);
                            break;
                        }
                    }
                    if let Some(id) = found {
                        Request::Clear(id)
                    } else {
                        self.stage = if self.texture.is_some() {
                            Stage::Append
                        } else {
                            Stage::Done
                        };
                        continue;
                    }
                }
                Stage::Destroy(id) => Request::Destroy(id),
                Stage::Append => {
                    let record = records.entries.len();
                    records.entries.push(Record::default());
                    records.accessories.push(record);
                    Request::Bind {
                        record,
                        name: self.name.clone(),
                        texture: self.texture.clone().unwrap(),
                    }
                }
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
    }
    pub fn acknowledge(&mut self, records: &mut Records) -> Result<(), String> {
        let request = self
            .pending
            .as_ref()
            .ok_or("accessory has no pending service")?;
        match *request {
            Request::Clear(id) | Request::Destroy(id) => {
                if records
                    .entries
                    .get(id)
                    .ok_or("accessory record absent")?
                    .binding
                    .is_some()
                {
                    return Err("accessory clear retained binding".into());
                }
                if matches!(request, Request::Destroy(_)) {
                    let i = records
                        .accessories
                        .iter()
                        .position(|&r| r == id)
                        .ok_or("accessory identity absent")?;
                    records.entries[id] = Record::default();
                    records.accessories.remove(i);
                    self.stage = Stage::Search;
                } else {
                    self.stage = Stage::Destroy(id);
                }
            }
            // Native add ignores the individual bind result. A failed lookup
            // leaves the newly appended record empty; asset-host errors may
            // instead keep the service pending for diagnosis/retry.
            Request::Bind { .. } => self.stage = Stage::Done,
        }
        self.pending = None;
        Ok(())
    }
}
