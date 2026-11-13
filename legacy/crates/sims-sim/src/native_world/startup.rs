use super::{arrays, fixed, resize, returned};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Matrix, Scratch, Stop};
use crate::saved_world::{self, Slot, Storage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod naming;
pub mod reader;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Event {
    Resize(usize),
    Naming(usize),
}

struct SpanContext {
    fixed: Vec<(u64, usize)>,
    embedded: BTreeSet<(u64, usize)>,
}

impl SpanContext {
    fn spans(
        &self,
        arrays: &arrays::State,
        child: &[(u64, usize)],
    ) -> Result<Vec<(u64, usize)>, Stop> {
        let dynamic = arrays.spans()?;
        let dynamic_set: BTreeSet<_> = dynamic.iter().copied().collect();
        let mut spans = self.fixed.clone();
        spans.extend(
            dynamic
                .into_iter()
                .filter(|span| !self.embedded.contains(span)),
        );
        spans.extend(
            child
                .iter()
                .copied()
                .filter(|span| !dynamic_set.contains(span)),
        );
        spans.sort_unstable();
        if spans
            .windows(2)
            .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
        {
            return Err(unsupported("raw startup aliases retained world ownership"));
        }
        Ok(spans)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingReturn {
    source_owner: Owner,
    source_tables: fixed::Tables,
    prior_resizes: usize,
    prior_namings: usize,
    returned: arrays::naming::Returned,
}

impl NamingReturn {
    pub fn source(&self) -> &arrays::State {
        self.returned.source()
    }
    pub fn arrays(&self) -> &arrays::State {
        self.returned.arrays()
    }
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn slot_offset(slot: Slot) -> Result<u8, Stop> {
    Ok(match slot {
        Slot::Altitude => 0x14,
        Slot::Ground => 0x18,
        Slot::Grass => 0x1c,
        Slot::TargetGrass => 0x20,
        Slot::Pool => 0x60,
        Slot::Water => 0x64,
        Slot::Covering(level) | Slot::Wall(level) | Slot::Head(level) | Slot::Flags(level)
            if level < 2 =>
        {
            let base = match slot {
                Slot::Covering(_) => 0x24,
                Slot::Wall(_) => 0x44,
                Slot::Head(_) => 0x3c,
                Slot::Flags(_) => 0x34,
                _ => unreachable!(),
            };
            base + level * 4
        }
        _ => return Err(unsupported("slot is not a supported saved-world matrix")),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    construction: returned::State,
    arrays: arrays::State,
    resizes: Vec<resize::State>,
    #[serde(default)]
    namings: Vec<NamingReturn>,
    #[serde(default)]
    history: Vec<Event>,
    fault: Option<Stop>,
}

impl World {
    pub fn from_returned(construction: returned::State) -> Result<Self, Stop> {
        construction.validate()?;
        let world = Self {
            arrays: construction.arrays().clone(),
            construction,
            resizes: vec![],
            namings: vec![],
            history: vec![],
            fault: None,
        };
        world.validate()?;
        Ok(world)
    }

    pub fn construction(&self) -> &returned::State {
        &self.construction
    }

    pub fn owner(&self) -> &Owner {
        self.resizes
            .last()
            .map_or_else(|| self.construction.owner(), resize::State::owner)
    }

    pub fn arrays(&self) -> &arrays::State {
        &self.arrays
    }

    pub fn active_size(&self) -> i32 {
        i32::from_le_bytes(self.owner().data[16..20].try_into().unwrap())
    }

    pub fn tables(&self) -> &fixed::Tables {
        self.resizes.last().map_or_else(
            || {
                &self
                    .construction
                    .population()
                    .handle()
                    .opened()
                    .context()
                    .resources
                    .resources
                    .lighting
                    .fixed
                    .tables
            },
            resize::State::tables,
        )
    }

    pub fn resizes(&self) -> &[resize::State] {
        &self.resizes
    }

    pub fn namings(&self) -> &[NamingReturn] {
        &self.namings
    }

    fn materialize_legacy_history(&mut self) {
        if self.history.is_empty() && self.namings.is_empty() {
            self.history
                .extend((0..self.resizes.len()).map(Event::Resize));
        }
    }

    fn accept_naming(mut self, returned: arrays::naming::Returned) -> Result<Self, Stop> {
        self.validate()?;
        returned.validate()?;
        if returned.source() != &self.arrays {
            return Err(unsupported(
                "naming does not consume the actual retained world arrays",
            ));
        }
        self.retained_spans_for(returned.arrays())?;
        let record = NamingReturn {
            source_owner: self.owner().clone(),
            source_tables: self.tables().clone(),
            prior_resizes: self.resizes.len(),
            prior_namings: self.namings.len(),
            returned,
        };
        self.materialize_legacy_history();
        self.arrays = record.arrays().clone();
        self.history.push(Event::Naming(self.namings.len()));
        self.namings.push(record);
        self.validate()?;
        Ok(self)
    }

    pub(super) fn accept_resize(mut self, state: resize::State) -> Result<Self, Stop> {
        self.validate()?;
        state.validate()?;
        if state.source_owner() != self.owner()
            || state.source_arrays() != &self.arrays
            || state.source_tables() != self.tables()
        {
            return Err(unsupported(
                "resize does not consume the actual retained world",
            ));
        }
        self.materialize_legacy_history();
        self.arrays = state.arrays().clone();
        self.history.push(Event::Resize(self.resizes.len()));
        self.resizes.push(state);
        self.validate()?;
        Ok(self)
    }

    pub fn retained_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.retained_spans_for(&self.arrays)
    }

    fn retained_spans_for(&self, arrays: &arrays::State) -> Result<Vec<(u64, usize)>, Stop> {
        self.span_context()?.spans(arrays, &[])
    }

    fn span_context(&self) -> Result<SpanContext, Stop> {
        let original: BTreeSet<_> = self.construction.arrays().spans()?.into_iter().collect();
        let mut spans = self.construction.population().retained_spans()?;
        let retained: BTreeSet<_> = spans.iter().copied().collect();
        let embedded: BTreeSet<_> = original.difference(&retained).copied().collect();
        spans.retain(|span| !original.contains(span));
        Ok(SpanContext {
            fixed: spans,
            embedded,
        })
    }

    pub fn validate(&self) -> Result<(), Stop> {
        if let Some(fault) = &self.fault {
            return Err(fault.clone());
        }
        self.construction.validate()?;
        let mut owner = self.construction.owner();
        let mut original = self.construction.arrays();
        let mut tables = &self
            .construction
            .population()
            .handle()
            .opened()
            .context()
            .resources
            .resources
            .lighting
            .fixed
            .tables;
        // Old resize-only snapshots retain their original implicit ordering.
        // Any naming history requires the explicit complete chronological log.
        let legacy;
        let history = if self.history.is_empty() && self.namings.is_empty() {
            legacy = (0..self.resizes.len())
                .map(Event::Resize)
                .collect::<Vec<_>>();
            &legacy
        } else {
            &self.history
        };
        let (mut resizes, mut namings) = (0, 0);
        let naming_context = self.span_context()?;
        for event in history {
            match *event {
                Event::Resize(index) if index == resizes => {
                    let state = self
                        .resizes
                        .get(index)
                        .ok_or_else(|| unsupported("raw startup resize history index is absent"))?;
                    state.validate()?;
                    state.validate_frames(self.retained_spans_for(state.source_arrays())?)?;
                    if state.source_owner() != owner || state.source_tables() != tables {
                        return Err(unsupported(
                            "raw startup lost its ordered resize provenance",
                        ));
                    }
                    Self::validate_saved_arrays(state.source_arrays(), original, owner)?;
                    owner = state.owner();
                    original = state.arrays();
                    tables = state.tables();
                    resizes += 1;
                }
                Event::Naming(index) if index == namings => {
                    let state = self
                        .namings
                        .get(index)
                        .ok_or_else(|| unsupported("raw startup naming history index is absent"))?;
                    state.returned.validate_in_context(|arrays, live| {
                        naming_context.spans(arrays, live).map(|_| ())
                    })?;
                    if &state.source_owner != owner
                        || &state.source_tables != tables
                        || state.prior_resizes != resizes
                        || state.prior_namings != namings
                    {
                        return Err(unsupported(
                            "raw startup naming lost its exact active owner or lifecycle position",
                        ));
                    }
                    Self::validate_saved_arrays(state.source(), original, owner)?;
                    self.retained_spans_for(state.arrays())?;
                    original = state.arrays();
                    namings += 1;
                }
                _ => {
                    return Err(unsupported(
                        "raw startup reordered or duplicated a lifecycle return",
                    ));
                }
            }
        }
        if resizes != self.resizes.len() || namings != self.namings.len() {
            return Err(unsupported("raw startup lost a resize or naming return"));
        }
        Self::validate_saved_arrays(&self.arrays, original, owner)?;
        self.retained_spans()?;
        Ok(())
    }

    fn validate_saved_arrays(
        arrays: &arrays::State,
        original: &arrays::State,
        owner: &Owner,
    ) -> Result<(), Stop> {
        arrays.validate_retained()?;
        if arrays.prefix != original.prefix
            || arrays.context != original.context
            || arrays.roots.len() != original.roots.len()
            || arrays.matrices.head != original.matrices.head
            || arrays.matrices.nodes.len() != original.matrices.nodes.len()
        {
            return Err(unsupported(
                "raw startup lost its retained constructor graph",
            ));
        }
        for (slot, root) in &arrays.roots {
            let previous = original
                .roots
                .get(slot)
                .ok_or_else(|| unsupported("raw startup replaced a constructor root"))?;
            if root.token != previous.token || root.data.len() != previous.data.len() {
                return Err(unsupported("raw startup replaced a constructor root owner"));
            }
            if root.data.len() == 40
                && (root.data[..4] != previous.data[..4] || root.data[36..] != previous.data[36..])
            {
                return Err(unsupported("raw startup changed a derived matrix owner"));
            }
            let identity = arrays.identity(*slot)?;
            let node = arrays
                .matrices
                .nodes
                .get(&identity)
                .ok_or_else(|| unsupported("raw startup lost a named matrix"))?;
            let earlier = original
                .matrices
                .nodes
                .get(&identity)
                .ok_or_else(|| unsupported("raw startup matrix has no constructor source"))?;
            if node.name != earlier.name || node.next != earlier.next {
                return Err(unsupported("raw startup changed an unbound naming owner"));
            }
            if [0x4c, 0x50, 0x58, 0x5c, 0x2c, 0x30].contains(slot) && node.matrix != earlier.matrix
            {
                return Err(unsupported(
                    "saved arrays changed a retained derived matrix",
                ));
            }
            let offset = usize::from(*slot);
            if owner.data[offset..offset + 4] != (root.token as u32).to_le_bytes() {
                return Err(unsupported(
                    "raw startup lost its actual world root pointer",
                ));
            }
        }
        for (current, earlier) in arrays.vectors.iter().zip(&original.vectors) {
            let current = current
                .as_ref()
                .ok_or_else(|| unsupported("raw startup lost its light vector"))?;
            let earlier = earlier
                .as_ref()
                .ok_or_else(|| unsupported("constructor light vector absent"))?;
            if current.owner != earlier.owner {
                return Err(unsupported("raw startup changed its light vector extent"));
            }
            let backing = current
                .backing
                .as_ref()
                .ok_or_else(|| unsupported("raw startup light backing absent"))?;
            let previous = earlier
                .backing
                .as_ref()
                .ok_or_else(|| unsupported("constructor light backing absent"))?;
            if backing.token != previous.token || backing.data.len() != previous.data.len() {
                return Err(unsupported("raw startup replaced its light allocation"));
            }
        }
        Ok(())
    }

    pub fn step_saved_arrays(
        &mut self,
        execution: &mut saved_world::Execution,
        scratch: &mut Scratch,
        host: &mut impl saved_world::Host,
    ) -> Result<Option<i32>, Stop> {
        match self.advance_saved_arrays(execution, scratch, host)? {
            saved_world::Advance::Pending => Ok(None),
            saved_world::Advance::Returned(status) if status != 0 => Ok(Some(status)),
            _ => Err(Stop::Unavailable(
                "raw World names and successful return require the owning saved-array reader"
                    .into(),
            )),
        }
    }

    fn advance_saved_arrays(
        &mut self,
        execution: &mut saved_world::Execution,
        scratch: &mut Scratch,
        host: &mut impl saved_world::Host,
    ) -> Result<saved_world::Advance, Stop> {
        self.validate()?;
        let active_size = self.active_size();
        let result = execution.advance(
            &mut Binding {
                arrays: &mut self.arrays,
                active_size,
            },
            scratch,
            host,
        );
        if let Err(fault) = self.arrays.synchronize().and_then(|_| self.validate()) {
            self.fault = Some(fault.clone());
            return Err(fault);
        }
        result
    }
}

struct Binding<'a> {
    arrays: &'a mut arrays::State,
    active_size: i32,
}

impl Storage for Binding<'_> {
    fn active_size(&self) -> i32 {
        self.active_size
    }

    fn matrix_mut(&mut self, slot: Slot) -> Result<&mut Matrix, Stop> {
        let identity = self.arrays.identity(slot_offset(slot)?)?;
        self.arrays
            .matrices
            .nodes
            .get_mut(&identity)
            .map(|node| &mut node.matrix)
            .ok_or_else(|| unsupported("raw startup saved matrix owner absent"))
    }

    fn reset_lights(&mut self, level: u8) -> Result<(), Stop> {
        let vector = self
            .arrays
            .vectors
            .get_mut(usize::from(level))
            .and_then(Option::as_mut)
            .ok_or_else(|| unsupported("raw startup light vector absent"))?;
        let backing = vector
            .backing
            .as_mut()
            .ok_or_else(|| unsupported("raw startup light allocation absent"))?;
        backing.data.fill(127);
        Ok(())
    }
}
