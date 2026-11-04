//! Original4764fe application poll. Named trees and current UI observations are
//! explicit synchronous services. A diagnostic yield never permits a lot tick.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Process word6c4db8, shared by rotation and scroll watches.
    pub baseline: i32,
}
impl Default for State {
    fn default() -> Self {
        Self { baseline: -1 }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    /// Original query mode1 reads CPState+230; retain the old wire tag.
    #[serde(rename = "ToolIdle")]
    PanelExpanded,
    Panel(String),
    Focus(i32),
    Portrait(i16),
    Rotation,
    Scroll,
    Control(i32),
    PanelAbsent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Native global12, even when there is no UI owner.
    PublishControl(i16),
    Query(ObjectId),
    Observe(Condition),
    Got(ObjectId),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    PanelExpansion(Option<i32>),
    Panel(Option<String>),
    Focus(Option<i32>),
    Portrait {
        focus_present: bool,
        focus_image: u32,
        resolved: Option<u32>,
    },
    Counter(i32),
    Control {
        image: Option<i32>,
        /// cTSWinBtn+e0 selection/state, not the enable byte at+168.
        state: u32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    /// False/missing named tree, or fresh object Temp0/1 after a true query.
    Query(Option<[i16; 2]>),
    Observed(Observation),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    owner: Option<ObjectId>,
    request: Option<Request>,
}
impl Poll {
    pub fn begin(owner: Option<ObjectId>, focused_image_id: Option<i32>) -> Result<Self, String> {
        if owner.is_some_and(|id| id <= 0) {
            return Err("invalid UI wait owner".into());
        }
        Ok(Self {
            owner,
            request: Some(Request::PublishControl(focused_image_id.unwrap_or(0) as i16)),
        })
    }
    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.request.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.owner.is_some_and(|id| id <= 0)
            || match &self.request {
                Some(Request::Query(id) | Request::Got(id)) => self.owner != Some(*id),
                Some(Request::Observe(_)) => self.owner.is_none(),
                _ => false,
            }
        {
            return Err("UI wait poll lost its captured owner".into());
        }
        Ok(())
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        expected: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        if self.request.as_ref() != Some(expected) {
            return Err("UI wait reply differs".into());
        }
        let next = match (expected, reply) {
            (Request::PublishControl(_), Reply::Done) => self.owner.map(Request::Query),
            (Request::Query(_), Reply::Query(None)) => {
                state.baseline = -1;
                None
            }
            (Request::Query(_), Reply::Query(Some([mode, parameter]))) => {
                let condition = match mode {
                    1 => Some(Condition::PanelExpanded),
                    2 => Some(Condition::Panel("motives".into())),
                    4 => Some(Condition::Focus(i32::from(parameter))),
                    5 => Some(Condition::Portrait(parameter)),
                    6 => Some(Condition::Rotation),
                    7 => Some(Condition::Scroll),
                    8 => Some(Condition::Control(i32::from(parameter))),
                    9 => Some(Condition::Panel("rel".into())),
                    11 => Some(Condition::Panel("job".into())),
                    13 => Some(Condition::Panel("skill".into())),
                    14 => Some(Condition::PanelAbsent),
                    15 => Some(Condition::Panel("per.ity".into())),
                    _ => None,
                };
                condition.map(Request::Observe)
            }
            (Request::Observe(condition), Reply::Observed(observed)) => {
                let matched = match (condition, observed) {
                    (Condition::PanelExpanded, Observation::PanelExpansion(value)) => {
                        value == Some(0)
                    }
                    (Condition::Panel(name), Observation::Panel(current)) => {
                        current.as_ref() == Some(name)
                    }
                    (Condition::PanelAbsent, Observation::Panel(current)) => current.is_none(),
                    (Condition::Focus(id), Observation::Focus(current)) => current == Some(*id),
                    (
                        Condition::Portrait(_),
                        Observation::Portrait {
                            focus_present,
                            focus_image,
                            resolved,
                        },
                    ) => focus_present && resolved == Some(focus_image),
                    (Condition::Control(id), Observation::Control { image, state }) => {
                        image == Some(*id) && state != 0
                    }
                    (Condition::Rotation | Condition::Scroll, Observation::Counter(current)) => {
                        if state.baseline == -1 {
                            state.baseline = current;
                            false
                        } else {
                            state.baseline != current
                        }
                    }
                    _ => return Err("UI wait observation kind differs".into()),
                };
                matched.then(|| Request::Got(self.owner.expect("validated UI wait owner")))
            }
            (Request::Got(_), Reply::Done) => {
                state.baseline = -1;
                None
            }
            _ => return Err("UI wait reply kind differs".into()),
        };
        self.request = next;
        Ok(())
    }
}
