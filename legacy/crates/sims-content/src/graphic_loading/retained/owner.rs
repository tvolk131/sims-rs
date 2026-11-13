//! One live allocation graph, with its Refresh history retained after unlink.
use super::*;
use crate::graphic_retirement::observed::{self, frame};

/// Legacy owners retain their existing wire shape. New phases use strict,
/// disjoint wrappers; a hybrid legacy/new payload must never discard history.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Owner(Phase);
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
enum Phase {
    Legacy(observed::Owner),
    Frame(Live),
    Retiring(AfterFrame),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Live {
    live_frame: frame::Owner,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AfterFrame {
    after_frame: frame::Retirement,
}
// Replay only the first outer key, forwarding values to the original map.
// Buffering an entire untagged owner loses JSON's numeric-map-key conversion.
struct Prefix<A> {
    first: Option<String>,
    map: A,
}
impl<'de, A: serde::de::MapAccess<'de>> serde::de::MapAccess<'de> for Prefix<A> {
    type Error = A::Error;
    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> std::result::Result<Option<K::Value>, Self::Error> {
        if let Some(key) = self.first.take() {
            seed.deserialize(serde::de::value::StringDeserializer::<A::Error>::new(key))
                .map(Some)
        } else {
            self.map.next_key_seed(seed)
        }
    }
    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> std::result::Result<V::Value, Self::Error> {
        self.map.next_value_seed(seed)
    }
}
impl<'de> Deserialize<'de> for Owner {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Owner;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a legacy retained owner or one strict frame phase")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Owner, A::Error> {
                use serde::de::Error;
                let first = map
                    .next_key::<String>()?
                    .ok_or_else(|| A::Error::custom("retained owner is empty"))?;
                let phase = match first.as_str() {
                    "live_frame" => Phase::Frame(Live {
                        live_frame: map.next_value()?,
                    }),
                    "after_frame" => Phase::Retiring(AfterFrame {
                        after_frame: map.next_value()?,
                    }),
                    _ => {
                        let legacy = observed::Owner::deserialize(
                            serde::de::value::MapAccessDeserializer::new(Prefix {
                                first: Some(first),
                                map,
                            }),
                        )?;
                        return Ok(Owner(Phase::Legacy(legacy)));
                    }
                };
                if map.next_key::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(A::Error::custom(
                        "retained frame phase has duplicate or mixed fields",
                    ));
                }
                Ok(Owner(phase))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
impl Owner {
    pub(super) fn legacy(input: Retained) -> Result<Self> {
        Ok(Self(Phase::Legacy(observed::Owner::from_retained(input)?)))
    }
    pub(super) fn live(input: frame::Input, binding: frame::Binding) -> Result<Self> {
        Ok(Self(Phase::Frame(Live {
            live_frame: frame::Owner::new(input, binding)?,
        })))
    }
    pub fn frame(&self) -> Option<&frame::Owner> {
        match &self.0 {
            Phase::Frame(v) => Some(&v.live_frame),
            _ => None,
        }
    }
    pub fn prelude(&self) -> Option<&frame::Owner> {
        match &self.0 {
            Phase::Frame(v) => Some(&v.live_frame),
            Phase::Retiring(v) => Some(v.after_frame.prelude()),
            _ => None,
        }
    }
    pub(super) fn retiring(&self) -> bool {
        matches!(self.0, Phase::Retiring(_))
    }
    pub fn destructor(&self) -> Option<&observed::Owner> {
        match &self.0 {
            Phase::Legacy(v) => Some(v),
            Phase::Retiring(v) => Some(v.after_frame.destructor()),
            Phase::Frame(_) => None,
        }
    }
    pub fn retained(&self) -> &Retained {
        match &self.0 {
            Phase::Legacy(v) => v.retained(),
            Phase::Frame(v) => &v.live_frame.input().retained,
            Phase::Retiring(v) => v.after_frame.retained(),
        }
    }
    pub fn complete(&self) -> bool {
        self.destructor().is_some_and(observed::Owner::complete)
    }
    pub fn outer_released(&self) -> bool {
        self.destructor()
            .is_some_and(observed::Owner::outer_released)
    }
    pub fn events(&self) -> &[observed::Event] {
        self.destructor().map_or(&[], observed::Owner::events)
    }
    pub fn service_count(&self) -> usize {
        self.destructor().map_or(0, observed::Owner::service_count)
    }
    pub fn last_service(&self) -> Option<Request> {
        self.destructor().and_then(observed::Owner::last_service)
    }
    pub fn global_focus(&self) -> Option<Id> {
        self.retained().global_focus
    }
    pub fn validate(&self) -> Result<()> {
        match &self.0 {
            Phase::Legacy(v) => v.validate(),
            Phase::Frame(v) => v.live_frame.validate(),
            Phase::Retiring(v) => v.after_frame.validate(),
        }
    }
    pub(super) fn frame_mut(&mut self) -> Result<&mut frame::Owner> {
        match &mut self.0 {
            Phase::Frame(v) => Ok(&mut v.live_frame),
            _ => anyhow::bail!("retained graphic has no live Refresh owner"),
        }
    }
    pub(super) fn begin_retirement(&mut self) -> Result<()> {
        self.validate()?;
        match &self.0 {
            Phase::Legacy(v) => {
                ensure!(v.events().is_empty(), "retained destructor already started")
            }
            Phase::Frame(v) => {
                let after_frame = v.live_frame.clone().into_retirement()?;
                self.0 = Phase::Retiring(AfterFrame { after_frame });
            }
            Phase::Retiring(_) => anyhow::bail!("retained destructor already started"),
        }
        Ok(())
    }
    pub(super) fn advance(&mut self) -> Result<bool> {
        match &mut self.0 {
            Phase::Legacy(v) => v.advance(),
            Phase::Retiring(v) => v.after_frame.advance(),
            Phase::Frame(_) => anyhow::bail!("retained Refresh has not handed off to destruction"),
        }
    }
    pub(super) fn release_animator(&mut self) -> Result<()> {
        match &mut self.0 {
            Phase::Legacy(v) => v.release_animator(),
            Phase::Retiring(v) => v.after_frame.release_animator(),
            Phase::Frame(_) => anyhow::bail!("live retained Animator cannot be released"),
        }
    }
}
