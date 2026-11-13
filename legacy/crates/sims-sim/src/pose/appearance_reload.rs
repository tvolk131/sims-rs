//! 5888d3 appearance reload and its 5887fc/588d11 record lifecycle.
//! Native selection and identity writes are ordered, resumable services. The
//! owner retains cache/attachment state; actual mesh geometry belongs to the host.
use super::{appearance::Records, appearance_selection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub selection: appearance_selection::State,
    pub model_present: bool,
    /// Native +250 auxiliary skin-layer binding and +254 metadata.
    pub attachment: Option<u64>,
    pub attachment_metadata: u32,
}
impl Owner {
    pub fn constructed(model_present: bool) -> Self {
        Self {
            selection: appearance_selection::State::constructed([0; 3]),
            model_present,
            attachment: None,
            attachment_metadata: 0,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ClearRecord(usize),
    Detach(u64),
    BindProfile,
    SelectAppearance,
    IdentityFields,
    /// Run record destruction (including clear) before removing the identity.
    DestroyAccessory(usize),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Bodies(usize),
    Hands(usize),
    Accessories(usize),
    Head,
    Attachment,
    Profile,
    ZeroHands,
    Select,
    Fields,
    Destroy(usize),
    Finish,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    Profile,
    Clear,
    DestroyAccessories,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reload {
    mode: Mode,
    stage: Stage,
    pending: Option<Request>,
    after: Stage,
}
impl Reload {
    pub fn begin(owner: &mut Owner) -> Self {
        owner.selection.dirty = true;
        Self {
            mode: Mode::Profile,
            stage: Stage::Bodies(0),
            pending: None,
            after: Stage::Done,
        }
    }
    /// 5887fc alone, without the enclosing reload's dirty/profile writes.
    pub fn clear() -> Self {
        Self {
            mode: Mode::Clear,
            stage: Stage::Bodies(0),
            pending: None,
            after: Stage::Done,
        }
    }
    /// 588d11 alone. Each record's destructor must finish before the next one.
    pub fn destroy_accessories() -> Self {
        Self {
            mode: Mode::DestroyAccessories,
            stage: Stage::Destroy(0),
            pending: None,
            after: Stage::Done,
        }
    }
    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Done && self.pending.is_none()
    }
    pub fn next(
        &mut self,
        owner: &mut Owner,
        records: &Records,
        person: &mut BTreeMap<i16, i16>,
    ) -> Result<Option<Request>, String> {
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let (request, after) = match self.stage {
                Stage::Bodies(i) => {
                    if let Some(&id) = records.body.get(i) {
                        (Request::ClearRecord(id), Stage::Bodies(i + 1))
                    } else {
                        self.stage = Stage::Hands(0);
                        continue;
                    }
                }
                Stage::Hands(i) => {
                    if let Some(&id) = records.hands.get(i) {
                        (Request::ClearRecord(id), Stage::Hands(i + 1))
                    } else {
                        self.stage = Stage::Accessories(0);
                        continue;
                    }
                }
                Stage::Accessories(i) => {
                    if let Some(&id) = records.accessories.get(i) {
                        (Request::ClearRecord(id), Stage::Accessories(i + 1))
                    } else {
                        self.stage = Stage::Head;
                        continue;
                    }
                }
                Stage::Head => (Request::ClearRecord(records.head), Stage::Attachment),
                Stage::Attachment => {
                    if let Some(id) = owner.attachment {
                        if !owner.model_present {
                            return Err("appearance attachment has no model owner".into());
                        }
                        (Request::Detach(id), Stage::Profile)
                    } else {
                        self.stage = Stage::Profile;
                        continue;
                    }
                }
                Stage::Profile => {
                    if self.mode == Mode::Clear {
                        self.stage = Stage::Done;
                        continue;
                    }
                    (Request::BindProfile, Stage::ZeroHands)
                }
                Stage::ZeroHands => {
                    if [22, 23].iter().any(|n| !person.contains_key(n)) {
                        return Err("appearance reload hand selectors absent".into());
                    }
                    person.insert(22, 0);
                    person.insert(23, 0);
                    self.stage = Stage::Select;
                    continue;
                }
                Stage::Select => {
                    if owner.model_present {
                        (Request::SelectAppearance, Stage::Fields)
                    } else {
                        self.stage = Stage::Fields;
                        continue;
                    }
                }
                Stage::Fields => (Request::IdentityFields, Stage::Destroy(0)),
                Stage::Destroy(i) => {
                    if let Some(&id) = records.accessories.get(i) {
                        (Request::DestroyAccessory(id), Stage::Destroy(i + 1))
                    } else {
                        self.stage = Stage::Finish;
                        continue;
                    }
                }
                Stage::Finish => {
                    if self.mode == Mode::Profile {
                        owner.selection.profile_reloaded();
                        owner.selection.dirty = true;
                    }
                    self.stage = Stage::Done;
                    continue;
                }
                Stage::Done => return Ok(None),
            };
            self.after = after;
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
    }
    pub fn acknowledge(&mut self, owner: &mut Owner, records: &mut Records) -> Result<(), String> {
        match self
            .pending
            .as_ref()
            .ok_or("appearance reload has no pending service")?
        {
            Request::Detach(_) => {
                owner.attachment = None;
                owner.attachment_metadata = 0;
            }
            Request::ClearRecord(id) | Request::DestroyAccessory(id) => {
                if records
                    .entries
                    .get(*id)
                    .ok_or("appearance record absent")?
                    .binding
                    .is_some()
                {
                    return Err("appearance clear service retained a binding".into());
                }
                // Preserve stable IDs as tombstones after native deallocation.
                // Clear the vector after visiting every live accessory identity.
                if matches!(self.pending, Some(Request::DestroyAccessory(_))) {
                    records.entries[*id] = Default::default();
                    if matches!(self.after, Stage::Destroy(i) if i == records.accessories.len()) {
                        records.accessories.clear();
                        self.after = Stage::Finish;
                    }
                }
            }
            _ => {}
        }
        self.pending = None;
        self.stage = self.after;
        Ok(())
    }
}
