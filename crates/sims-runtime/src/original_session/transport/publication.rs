//! Fresh transport calls use the shared4ce44f kernel. Already-saved partial
//! callers retain their exact representation and continuation on cold restore.
use serde::{Deserialize, Serialize};
use sims_vm::interface::{state::State, transport as old, world_publication as world};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(super) enum Publication {
    Shared { world: world::Update },
    Legacy(old::Update),
}
fn request(r: old::UpdateRequest) -> world::Request {
    match r {
        old::UpdateRequest::SetSpeed(v) => world::Request::SetSpeed(v),
        old::UpdateRequest::Pause => world::Request::Pause,
        old::UpdateRequest::Resume => world::Request::Resume,
        old::UpdateRequest::PauseAudio => world::Request::PauseAudio,
        old::UpdateRequest::ResumeAudio => world::Request::ResumeAudio,
    }
}
fn reply(r: world::Request) -> Result<old::UpdateRequest, String> {
    Ok(match r {
        world::Request::SetSpeed(v) => old::UpdateRequest::SetSpeed(v),
        world::Request::Pause => old::UpdateRequest::Pause,
        world::Request::Resume => old::UpdateRequest::Resume,
        world::Request::PauseAudio => old::UpdateRequest::PauseAudio,
        world::Request::ResumeAudio => old::UpdateRequest::ResumeAudio,
        _ => return Err("transport received an unrelated world-publication service".into()),
    })
}
impl Publication {
    pub fn new() -> Self {
        Self::Shared {
            world: world::Update::new(old::DIRTY),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Shared { world } => {
                world.validate()?;
                if world.mask() != old::DIRTY {
                    return Err("transport publication mask differs".into());
                }
                if let Some(r) = world.request() {
                    reply(r)?;
                }
                Ok(())
            }
            Self::Legacy(c) => c.validate(),
        }
    }
    pub fn request(&self) -> Option<world::Request> {
        match self {
            Self::Shared { world } => world.request(),
            Self::Legacy(c) => c.request().map(request),
        }
    }
    pub fn acknowledge(&mut self, r: old::UpdateRequest) -> Result<(), String> {
        self.validate()?;
        match self {
            Self::Shared { world } => world.acknowledge(request(r)),
            Self::Legacy(c) => c.acknowledge(r),
        }
    }
    pub fn advance(
        &mut self,
        s: &State,
        mode: &mut i16,
        input: old::Inputs,
    ) -> Result<Option<old::UpdateRequest>, String> {
        self.validate()?;
        match self {
            Self::Shared { world } => world
                .advance_transport(s, mode, input)?
                .map(reply)
                .transpose(),
            Self::Legacy(c) => c.advance(s, mode, input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_partial_publication_stays_legacy_while_new_calls_share_world_kernel() {
        let mut s = State::constructed(924);
        s.mode = 2;
        let input = old::Inputs {
            menu_present: false,
            menu_active: 0,
            closing: 0,
            lot_speed: 760,
        };
        let mut mode = 3;
        let mut legacy = old::Update::new(old::DIRTY).unwrap();
        let first = legacy.advance(&s, &mut mode, input).unwrap().unwrap();
        let saved = serde_json::to_value(legacy).unwrap();
        let mut restored: Publication = serde_json::from_value(saved.clone()).unwrap();
        assert!(matches!(restored, Publication::Legacy(_)));
        assert_eq!(serde_json::to_value(&restored).unwrap(), saved);
        assert_eq!(restored.advance(&s, &mut mode, input).unwrap(), Some(first));
        let mut fresh = Publication::new();
        assert_eq!(fresh.advance(&s, &mut mode, input).unwrap(), Some(first));
        assert!(serde_json::to_value(&fresh).unwrap().get("world").is_some());
        for next in [
            Some(old::UpdateRequest::Resume),
            Some(old::UpdateRequest::ResumeAudio),
            None,
        ] {
            for c in [&mut restored, &mut fresh] {
                let r = reply(c.request().unwrap()).unwrap();
                c.acknowledge(r).unwrap();
                assert_eq!(c.advance(&s, &mut mode, input).unwrap(), next);
            }
        }
        let mut bad = serde_json::to_value(Publication::new()).unwrap();
        bad["world"]["mask"] = serde_json::json!(u32::MAX);
        let mut bad: Publication = serde_json::from_value(bad).unwrap();
        let before = bad.clone();
        let mode_before = mode;
        assert!(bad.advance(&s, &mut mode, input).is_err());
        assert_eq!(bad, before);
        assert_eq!(mode, mode_before);
    }
}
