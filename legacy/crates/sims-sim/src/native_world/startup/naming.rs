//! Own the quiescent World and its live naming child until the child returns.
use super::{World, unsupported};
use crate::{native_matrix, native_world::arrays, saved_matrix::Stop};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug)]
pub struct BeginError {
    pub stop: Stop,
    pub source: World,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Owner {
    source: World,
    child: arrays::naming::Execution,
}

impl<'de> Deserialize<'de> for Owner {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Retained {
            source: World,
            child: arrays::naming::Execution,
        }
        let value = Retained::deserialize(deserializer)?;
        let owner = Self {
            source: value.source,
            child: value.child,
        };
        owner.validate().map_err(serde::de::Error::custom)?;
        Ok(owner)
    }
}

impl Owner {
    pub fn new(source: World) -> Result<Self, BeginError> {
        if let Err(stop) = source.validate() {
            return Err(BeginError { stop, source });
        }
        let child = match arrays::naming::Execution::new(source.arrays().clone()) {
            Ok(child) => child,
            Err(error) => {
                return Err(BeginError {
                    stop: error.stop,
                    source,
                });
            }
        };
        Ok(Self { source, child })
    }

    /// Historical, read-only World; current arrays are exposed separately.
    pub fn source(&self) -> &World {
        &self.source
    }
    pub fn child(&self) -> &arrays::naming::Execution {
        &self.child
    }
    pub fn arrays(&self) -> &arrays::State {
        self.child.arrays()
    }
    pub fn complete(&self) -> bool {
        self.child.complete()
    }

    pub fn retained_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        self.source
            .span_context()?
            .spans(self.child.arrays(), &self.child.retained_spans()?)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.source.validate()?;
        if self.child.source() != self.source.arrays() {
            return Err(unsupported(
                "World naming child lost its exact source graph",
            ));
        }
        let context = self.source.span_context()?;
        self.child
            .validate_in_context(|arrays, live| context.spans(arrays, live).map(|_| ()))?;
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl native_matrix::Host) -> Result<bool, Stop> {
        self.validate()?;
        let result = self.child.step(host);
        self.validate()?;
        result
    }

    pub fn into_world(self) -> Result<World, Self> {
        if !self.complete() || self.validate().is_err() {
            return Err(self);
        }
        let returned = match self.child.clone().into_returned() {
            Ok(returned) => returned,
            Err(_) => return Err(self),
        };
        match self.source.clone().accept_naming(returned) {
            Ok(world) => Ok(world),
            Err(_) => Err(self),
        }
    }
}
