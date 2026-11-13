//! Retained execution of the same 18-name sequence used by World construction.
//! This child owns its source arrays, partial relinking, temporary strings and
//! successful service replies. It does not admit a native World or bypass the
//! startup World's separate request/return provenance checks.
use super::{NAMES, State};
use crate::native_matrix::{self, Naming};
use crate::native_world::validate_owner;
use crate::native_world_children::Owner;
use crate::saved_matrix::{self, Allocated, AllocationRequest, Lookup, MAX_ALLOCATION_BYTES, Stop};
use serde::{Deserialize, Deserializer, Serialize};

const MAX_STEPS: usize = 16_384;

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Allocate,
    Accept,
    Name,
    Release,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    arrays: State,
    ordinal: usize,
    phase: Phase,
    pending: Option<Allocated>,
    temporary: Option<Owner>,
    naming: Option<Naming>,
}

impl Progress {
    fn new(arrays: State) -> Self {
        Self {
            arrays,
            ordinal: 0,
            phase: Phase::Allocate,
            pending: None,
            temporary: None,
            naming: None,
        }
    }

    fn validate(&self) -> Result<(), Stop> {
        self.arrays.validate_retained()?;
        if self.ordinal > NAMES.len()
            || (self.ordinal == NAMES.len()) != (self.phase == Phase::Complete)
            || (self.phase == Phase::Accept) != self.pending.is_some()
            || matches!(self.phase, Phase::Name | Phase::Release) != self.temporary.is_some()
            || (self.phase == Phase::Name) != self.naming.is_some()
        {
            return Err(unsupported(
                "world naming cursor lost its source or pending owners",
            ));
        }
        if let Some(naming) = &self.naming {
            let (slot, input) = NAMES[self.ordinal];
            naming.validate_for(&self.arrays.matrices, self.arrays.identity(slot)?, input)?;
        }
        let mut spans = self.arrays.spans()?;
        let mut introduced = Vec::new();
        if let Some(temporary) = &self.temporary {
            validate_owner(temporary, NAMES[self.ordinal].1.len() + 1)?;
            if temporary.data[..temporary.data.len() - 1] != *NAMES[self.ordinal].1
                || temporary.data.last() != Some(&0)
            {
                return Err(unsupported(
                    "world naming temporary lost its actual C-string input",
                ));
            }
            introduced.push((temporary.token, temporary.data.len()));
        }
        if let Some(Allocated::Bytes { token, data }) = &self.pending {
            validate_owner(
                &Owner {
                    token: *token,
                    data: data.clone(),
                },
                data.len(),
            )?;
            introduced.push((*token, data.len()));
        }
        if let Some(naming) = &self.naming {
            introduced.extend(naming.retained_spans());
        }
        // Check every newly returned name buffer before a later transition can
        // free its predecessor. Naming's generic Name carrier does not itself
        // enforce native-width pointers or representable capacity ends.
        for (token, bytes) in &introduced {
            if *token == 0
                || *bytes == 0
                || *bytes > MAX_ALLOCATION_BYTES
                || token
                    .checked_add(*bytes as u64)
                    .is_none_or(|end| end > u64::from(u32::MAX))
            {
                return Err(unsupported(
                    "world naming pending buffer exceeds native pointer bounds",
                ));
            }
        }
        if let Some(game) = self.arrays.prefix.game() {
            for (token, bytes) in &introduced {
                game.guard_span(*token, *bytes)?;
            }
        }
        // arrays.spans() already includes established Game/source owners.
        // Only newly introduced child buffers are checked against that source;
        // checking established spans against themselves rejects a valid graph.
        spans.extend(introduced);
        spans.sort_unstable();
        if spans
            .windows(2)
            .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
        {
            return Err(unsupported(
                "world naming aliases retained array or pending ownership",
            ));
        }
        Ok(())
    }

    fn step(&mut self, host: &mut impl native_matrix::Host) -> Result<bool, Stop> {
        match self.phase {
            Phase::Allocate => {
                self.pending = Some(host.allocate_name((NAMES[self.ordinal].1.len() + 1) as u32)?);
                self.phase = Phase::Accept;
            }
            Phase::Accept => {
                let (slot, input) = NAMES[self.ordinal];
                let (token, data) = match self.pending.as_ref() {
                    Some(Allocated::Bytes { token, data }) if data.len() == input.len() + 1 => {
                        (*token, data)
                    }
                    Some(Allocated::Null) => {
                        return Err(Stop::Unavailable(
                            "world naming temporary returned null".into(),
                        ));
                    }
                    _ => {
                        return Err(unsupported(
                            "world naming temporary has the wrong allocation extent",
                        ));
                    }
                };
                let mut temporary = Owner {
                    token,
                    data: data.clone(),
                };
                validate_owner(&temporary, input.len() + 1)?;
                if token + temporary.data.len() as u64 > u64::from(u32::MAX) {
                    return Err(unsupported(
                        "world naming temporary end exceeds native pointer width",
                    ));
                }
                temporary.data[..input.len()].copy_from_slice(input);
                temporary.data[input.len()] = 0;
                self.naming = Some(Naming::external(
                    self.arrays.identity(slot)?,
                    input.to_vec(),
                )?);
                self.temporary = Some(temporary);
                self.pending = None;
                self.phase = Phase::Name;
            }
            Phase::Name => {
                if self
                    .naming
                    .as_mut()
                    .ok_or_else(|| unsupported("world naming child absent"))?
                    .step(&mut self.arrays.matrices, host)?
                {
                    self.naming = None;
                    self.phase = Phase::Release;
                }
            }
            Phase::Release => {
                host.release_name(
                    self.temporary
                        .as_ref()
                        .ok_or_else(|| unsupported("world naming temporary absent"))?
                        .token,
                )?;
                self.temporary = None;
                self.ordinal += 1;
                self.phase = if self.ordinal == NAMES.len() {
                    Phase::Complete
                } else {
                    Phase::Allocate
                };
            }
            Phase::Complete => {}
        }
        self.arrays.synchronize()?;
        Ok(self.phase == Phase::Complete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Effect {
    Allocate { bytes: u32, reply: Allocated },
    Release { token: u64 },
}

struct Recording<'a, H> {
    host: &'a mut H,
    effects: &'a mut Vec<Effect>,
}

struct Replay<'a> {
    effects: &'a [Effect],
    next: usize,
}

macro_rules! no_matrix_source {
    () => {
        fn lookup(&mut self, _: u32, _: u32, _: u32) -> Result<Lookup, Stop> {
            Err(unsupported("world naming cannot read saved resources"))
        }
        fn source(&mut self, _: u64) -> Result<Vec<u8>, Stop> {
            Err(unsupported("world naming has no source lease"))
        }
        fn release(&mut self, _: u64) -> Result<(), Stop> {
            Err(unsupported("world naming cannot release a source lease"))
        }
        fn free(&mut self, _: u64) -> Result<(), Stop> {
            Err(unsupported("world naming cannot free a matrix backing"))
        }
        fn allocate(&mut self, _: AllocationRequest) -> Result<Allocated, Stop> {
            Err(unsupported("world naming cannot allocate a matrix backing"))
        }
    };
}

impl<H: native_matrix::Host> saved_matrix::Host for Recording<'_, H> {
    no_matrix_source!();
}
impl saved_matrix::Host for Replay<'_> {
    no_matrix_source!();
}

impl<H: native_matrix::Host> native_matrix::Host for Recording<'_, H> {
    fn require_owner(&self, _: u64, _: u32) -> Result<(), Stop> {
        Err(unsupported(
            "world naming cannot construct a replacement matrix",
        ))
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        let reply = self.host.allocate_name(bytes)?;
        self.effects.push(Effect::Allocate {
            bytes,
            reply: reply.clone(),
        });
        Ok(reply)
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        self.host.release_name(token)?;
        self.effects.push(Effect::Release { token });
        Ok(())
    }
}

impl native_matrix::Host for Replay<'_> {
    fn require_owner(&self, _: u64, _: u32) -> Result<(), Stop> {
        Err(unsupported(
            "world naming replay cannot construct a replacement matrix",
        ))
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        match self.effects.get(self.next) {
            Some(Effect::Allocate {
                bytes: expected,
                reply,
            }) if *expected == bytes => {
                self.next += 1;
                Ok(reply.clone())
            }
            _ => Err(unsupported(
                "world naming replay lost an exact allocator reply",
            )),
        }
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        match self.effects.get(self.next) {
            Some(Effect::Release { token: expected }) if *expected == token => {
                self.next += 1;
                Ok(())
            }
            _ => Err(unsupported("world naming replay lost its exact release")),
        }
    }
}

#[derive(Debug)]
pub struct BeginError {
    pub stop: Stop,
    pub source: State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Execution {
    source: State,
    progress: Progress,
    steps: usize,
    effects: Vec<Effect>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    source: State,
    progress: Progress,
    steps: usize,
    effects: Vec<Effect>,
}

impl<'de> Deserialize<'de> for Execution {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Retained::deserialize(deserializer)?;
        let execution = Self {
            source: value.source,
            progress: value.progress,
            steps: value.steps,
            effects: value.effects,
        };
        execution.validate().map_err(serde::de::Error::custom)?;
        Ok(execution)
    }
}

impl Execution {
    pub fn new(source: State) -> Result<Self, BeginError> {
        if let Err(stop) = source.validate_retained() {
            return Err(BeginError { stop, source });
        }
        Ok(Self {
            progress: Progress::new(source.clone()),
            source,
            steps: 0,
            effects: vec![],
        })
    }

    pub fn source(&self) -> &State {
        &self.source
    }
    pub fn arrays(&self) -> &State {
        &self.progress.arrays
    }
    pub fn completed_names(&self) -> usize {
        self.progress.ordinal
    }
    pub fn complete(&self) -> bool {
        self.progress.phase == Phase::Complete
    }

    /// Current live graph plus temporary/nested buffers. Immutable source and
    /// historical effect images are excluded from live allocation occupancy.
    pub fn retained_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        Self::live_spans(&self.progress)
    }

    fn live_spans(progress: &Progress) -> Result<Vec<(u64, usize)>, Stop> {
        let mut spans = progress.arrays.spans()?;
        if let Some(owner) = &progress.temporary {
            spans.push((owner.token, owner.data.len()));
        }
        if let Some(Allocated::Bytes { token, data }) = &progress.pending {
            spans.push((*token, data.len()));
        }
        if let Some(naming) = &progress.naming {
            spans.extend(naming.retained_spans());
        }
        spans.sort_unstable();
        Ok(spans)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.validate_in_context(|_, _| Ok(()))
    }

    /// The enclosing World supplies its retained non-array closure. Validate
    /// that context at every replayed boundary, including already-freed temps.
    pub(crate) fn validate_in_context(
        &self,
        mut check: impl FnMut(&State, &[(u64, usize)]) -> Result<(), Stop>,
    ) -> Result<(), Stop> {
        self.source.validate_retained()?;
        if self.steps > MAX_STEPS || self.effects.len() > MAX_STEPS {
            return Err(unsupported(
                "world naming exceeds the bounded continuation profile",
            ));
        }
        let retained_bytes = self.effects.iter().try_fold(0_usize, |sum, effect| {
            sum.checked_add(match effect {
                Effect::Allocate {
                    reply: Allocated::Bytes { data, .. },
                    ..
                } => data.len(),
                _ => 0,
            })
        });
        if retained_bytes.is_none_or(|bytes| bytes > MAX_ALLOCATION_BYTES) {
            return Err(unsupported(
                "world naming allocator history exceeds the bounded profile",
            ));
        }
        let mut expected = Progress::new(self.source.clone());
        let mut replay = Replay {
            effects: &self.effects,
            next: 0,
        };
        expected.validate()?;
        check(&expected.arrays, &Self::live_spans(&expected)?)?;
        for _ in 0..self.steps {
            if expected.phase == Phase::Complete {
                return Err(unsupported("world naming retained steps beyond its return"));
            }
            expected.step(&mut replay)?;
            expected.validate()?;
            check(&expected.arrays, &Self::live_spans(&expected)?)?;
        }
        self.progress.validate()?;
        if replay.next != self.effects.len() || expected != self.progress {
            return Err(unsupported(
                "world naming differs from its exact source and service history",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl native_matrix::Host) -> Result<bool, Stop> {
        self.validate()?;
        if self.complete() {
            return Ok(true);
        }
        if self.steps == MAX_STEPS {
            return Err(Stop::Unavailable(
                "world naming step profile exhausted".into(),
            ));
        }
        let complete = self.progress.step(&mut Recording {
            host,
            effects: &mut self.effects,
        })?;
        self.steps += 1;
        self.validate()?;
        Ok(complete)
    }

    /// An incomplete or invalid child stays owned by the caller. A returned
    /// child retains its full source/effect history for a later World handoff.
    pub fn into_returned(self) -> Result<Returned, Self> {
        if self.complete() && self.validate().is_ok() {
            Ok(Returned { execution: self })
        } else {
            Err(self)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Returned {
    execution: Execution,
}

impl<'de> Deserialize<'de> for Returned {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RetainedReturn {
            execution: Execution,
        }
        let returned = Self {
            execution: RetainedReturn::deserialize(deserializer)?.execution,
        };
        returned.validate().map_err(serde::de::Error::custom)?;
        Ok(returned)
    }
}

impl Returned {
    pub(crate) fn validate_in_context(
        &self,
        check: impl FnMut(&State, &[(u64, usize)]) -> Result<(), Stop>,
    ) -> Result<(), Stop> {
        self.execution.validate_in_context(check)?;
        if !self.execution.complete() {
            return Err(unsupported("world naming child has not returned"));
        }
        Ok(())
    }
    pub fn source(&self) -> &State {
        self.execution.source()
    }
    pub fn arrays(&self) -> &State {
        self.execution.arrays()
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.execution.validate()?;
        if !self.execution.complete() {
            return Err(unsupported("world naming child has not returned"));
        }
        Ok(())
    }
}
