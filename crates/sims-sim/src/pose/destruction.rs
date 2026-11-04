//! 588080, 5881fe and 584898 destruction order. Native pointer identities are
//! represented by stable host IDs. Each returned service must finish before
//! its corresponding pointer is cleared; callbacks may mutate later inputs.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub model: Option<u64>,
    pub profile: Option<u64>,
    /// Foreground, outgoing blend, carry, base.
    pub layers: [Option<u64>; 4],
    /// Native +14c, +144, +140, +148, in destruction order.
    pub anchors: [Option<u64>; 4],
    pub head: Option<u64>,
    pub accessories: Vec<Option<u64>>,
    pub hands: Vec<Option<u64>>,
    pub bodies: Vec<Option<u64>>,
    pub generic: Vec<Option<u64>>,
    pub selected: bool,
    pub focus: bool,
    pub base_focus: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Storage {
    Accessories,
    Hands,
    Bodies,
    Name,
    BaseAuxiliaryRecords,
    Generic,
    BaseLayers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Reset,
    ClearAppearances,
    ReleaseProfile(u64),
    DetachLayer(u64),
    DestroyAnchor(u64),
    DestroyModel(u64),
    ClearSelected,
    ClearFocus,
    DestroyRecord(u64),
    Storage(Storage),
    BaseReset,
    ClearBaseFocus,
    BaseClear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Reset,
    Clear,
    Profile,
    Layers(usize),
    Anchors(usize),
    Model,
    Selected,
    Focus,
    Head,
    Accessories(usize),
    Hands(usize),
    Bodies(usize),
    Storage(usize),
    BaseReset,
    BaseFocus,
    BaseClear,
    Generic(usize),
    BaseStorage(usize),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    stage: Stage,
    pending: Option<Request>,
}
impl Default for Execution {
    fn default() -> Self {
        Self {
            stage: Stage::Reset,
            pending: None,
        }
    }
}
impl Execution {
    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Done && self.pending.is_none()
    }
    pub fn pending_request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }

    pub fn validate(&self, s: &State) -> Result<(), String> {
        let valid = match self.stage {
            Stage::Layers(i) | Stage::Anchors(i) => i <= 4,
            Stage::Accessories(i) => i <= s.accessories.len(),
            Stage::Hands(i) => i <= s.hands.len(),
            Stage::Bodies(i) => i <= s.bodies.len(),
            Stage::Generic(i) => i <= s.generic.len(),
            Stage::Storage(i) => i < 3,
            Stage::BaseStorage(i) => i < 4,
            _ => true,
        };
        if !valid
            || self
                .pending
                .as_ref()
                .is_some_and(|r| self.request(s).as_ref() != Some(r))
        {
            return Err("invalid animator destruction continuation".into());
        }
        Ok(())
    }

    fn request(&self, s: &State) -> Option<Request> {
        use Request as R;
        Some(match self.stage {
            Stage::Reset => R::Reset,
            Stage::Clear => R::ClearAppearances,
            Stage::Profile => R::ReleaseProfile(s.profile?),
            Stage::Layers(i) => R::DetachLayer(if s.model.is_some() {
                *s.layers.get(i)?
            } else {
                None
            }?),
            Stage::Anchors(i) => R::DestroyAnchor((*s.anchors.get(i)?)?),
            Stage::Model => R::DestroyModel(s.model?),
            Stage::Selected => {
                if s.selected {
                    R::ClearSelected
                } else {
                    return None;
                }
            }
            Stage::Focus => {
                if s.focus {
                    R::ClearFocus
                } else {
                    return None;
                }
            }
            Stage::Head => R::DestroyRecord(s.head?),
            Stage::Accessories(i) => R::DestroyRecord((*s.accessories.get(i)?)?),
            Stage::Hands(i) => R::DestroyRecord((*s.hands.get(i)?)?),
            Stage::Bodies(i) => R::DestroyRecord((*s.bodies.get(i)?)?),
            Stage::Storage(i) => {
                R::Storage(*[Storage::Accessories, Storage::Hands, Storage::Bodies].get(i)?)
            }
            Stage::BaseReset => R::BaseReset,
            Stage::BaseFocus => {
                if s.base_focus {
                    R::ClearBaseFocus
                } else {
                    return None;
                }
            }
            Stage::BaseClear => R::BaseClear,
            Stage::Generic(i) => R::DestroyRecord((*s.generic.get(i)?)?),
            Stage::BaseStorage(i) => R::Storage(
                *[
                    Storage::Name,
                    Storage::BaseAuxiliaryRecords,
                    Storage::Generic,
                    Storage::BaseLayers,
                ]
                .get(i)?,
            ),
            Stage::Done => return None,
        })
    }

    fn advance(&mut self, s: &mut State) {
        self.stage = match self.stage {
            Stage::Reset => Stage::Clear,
            Stage::Clear => Stage::Profile,
            Stage::Profile => Stage::Layers(0),
            Stage::Layers(i) if i < 4 => {
                if s.model.is_some() {
                    s.layers[i] = None;
                }
                Stage::Layers(i + 1)
            }
            Stage::Layers(_) => Stage::Anchors(0),
            Stage::Anchors(i) if i < 4 => {
                s.anchors[i] = None;
                Stage::Anchors(i + 1)
            }
            Stage::Anchors(_) => Stage::Model,
            Stage::Model => {
                s.model = None;
                Stage::Selected
            }
            Stage::Selected => {
                s.selected = false;
                Stage::Focus
            }
            Stage::Focus => {
                s.focus = false;
                Stage::Head
            }
            Stage::Head => {
                s.head = None;
                Stage::Accessories(0)
            }
            Stage::Accessories(i) if i < s.accessories.len() => {
                s.accessories[i] = None;
                Stage::Accessories(i + 1)
            }
            Stage::Accessories(_) => {
                s.accessories.clear();
                Stage::Hands(0)
            }
            Stage::Hands(i) if i < s.hands.len() => {
                s.hands[i] = None;
                Stage::Hands(i + 1)
            }
            Stage::Hands(_) => Stage::Bodies(0),
            Stage::Bodies(i) if i < s.bodies.len() => {
                s.bodies[i] = None;
                Stage::Bodies(i + 1)
            }
            Stage::Bodies(_) => Stage::Storage(0),
            Stage::Storage(i) if i < 2 => Stage::Storage(i + 1),
            Stage::Storage(_) => Stage::BaseReset,
            Stage::BaseReset => Stage::BaseFocus,
            Stage::BaseFocus => {
                s.base_focus = false;
                Stage::BaseClear
            }
            Stage::BaseClear => Stage::Generic(0),
            Stage::Generic(i) if i < s.generic.len() => {
                s.generic[i] = None;
                Stage::Generic(i + 1)
            }
            Stage::Generic(_) => Stage::BaseStorage(0),
            Stage::BaseStorage(i) if i < 3 => Stage::BaseStorage(i + 1),
            Stage::BaseStorage(_) | Stage::Done => Stage::Done,
        };
    }

    pub fn next(&mut self, s: &mut State) -> Result<Option<Request>, String> {
        self.validate(s)?;
        if let Some(pending) = &self.pending {
            if self.request(s).as_ref() != Some(pending) {
                return Err("animator destruction pending owner changed".into());
            }
            return Ok(Some(pending.clone()));
        }
        while self.stage != Stage::Done {
            if let Some(request) = self.request(s) {
                self.pending = Some(request.clone());
                return Ok(Some(request));
            }
            self.advance(s);
        }
        Ok(None)
    }

    pub fn acknowledge(&mut self, s: &mut State, expected: &Request) -> Result<(), String> {
        self.validate(s)?;
        if self.pending.as_ref() != Some(expected) || self.request(s).as_ref() != Some(expected) {
            return Err("stale animator destruction acknowledgment".into());
        }
        self.advance(s);
        self.pending = None;
        Ok(())
    }
}
