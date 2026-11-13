//! Explicit53a01f callback body, not a generic graphics pump or render getter.
//! Logical GUI identities are replacement-owned, never native heap addresses.
use super::Session;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::{World, family_label as native};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplicationFamily {
    Unknown,
    KnownNull,
    Family(i16),
}

/// Supported observed nonallocating string-storage path. Historical absence
/// never supplies capacity or this source profile automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageProfile {
    CompleteCollection191Inline256Observed20261025,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountedBinding {
    pub bytes: Vec<u8>,
    pub capacity: Option<u16>,
    pub profile: Option<StorageProfile>,
}

/// Explicit construction of NEW replacement logical text owners. An absent
/// field is unknown, Some(0) is known null; neither is filled from a save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub fields: native::Fields,
    pub application_family: ApplicationFamily,
    pub counted_text: BTreeMap<u32, CountedBinding>,
    pub label_text: BTreeMap<u32, Vec<u8>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Counted {
    length: u32,
    buffer: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capacity: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<StorageProfile>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Buffer {
    state: u32,
    bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Execution {
    tick: u64,
    serial: u64,
    raw_argument: u32,
    native: native::Call,
    pending_request: Option<native::Request>,
    ready_reply: Option<native::Reply>,
    finalized: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    fields: native::Fields,
    application_family: ApplicationFamily,
    counted: BTreeMap<u32, Counted>,
    buffers: BTreeMap<u32, Buffer>,
    next_buffer: u32,
    labels: BTreeMap<u32, Vec<u8>>,
    /// These are actual independent temporary byte owners, not success markers.
    temporaries: BTreeMap<u8, Vec<u8>>,
    created: u8,
    destroyed: Vec<u8>,
    serial: u64,
    call: Option<Execution>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Service(native::Request),
    Complete,
}

fn string(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= i32::MAX as usize && !bytes.contains(&0),
        "unsupported logical CString bytes"
    );
    Ok(())
}
impl State {
    pub fn bind(binding: Binding) -> Result<Self> {
        ensure!(binding.fields.owner != 0, "family window identity absent");
        ensure!(
            binding
                .counted_text
                .keys()
                .chain(binding.label_text.keys())
                .all(|id| *id != 0),
            "null logical text owner"
        );
        let mut state = Self {
            fields: binding.fields,
            application_family: binding.application_family,
            counted: BTreeMap::new(),
            buffers: BTreeMap::new(),
            next_buffer: 1,
            labels: binding.label_text,
            temporaries: BTreeMap::new(),
            created: 0,
            destroyed: vec![],
            serial: 0,
            call: None,
        };
        for (id, text) in binding.counted_text {
            state.allocate_buffer(id, text)?;
        }
        state.validate_owned()?;
        Ok(state)
    }
    pub fn pending(&self) -> bool {
        self.call.as_ref().is_some_and(|c| !c.finalized)
    }
    pub fn fields(&self) -> native::Fields {
        self.fields
    }
    pub fn replace_fields(&mut self, fields: native::Fields) -> Result<()> {
        ensure!(
            fields.owner == self.fields.owner,
            "family window identity changed"
        );
        self.fields = fields;
        Ok(())
    }
    pub fn set_application_family(&mut self, value: ApplicationFamily) {
        self.application_family = value;
    }
    /// Pure completed-label projection; never calls the family getter.
    pub fn label(&self, owner: u32) -> Option<&[u8]> {
        self.labels.get(&owner).map(Vec::as_slice)
    }
    pub fn destruction_order(&self) -> &[u8] {
        &self.destroyed
    }
    pub fn result(&self) -> Option<u8> {
        self.call.as_ref()?.native.result()
    }
    pub fn counted_buffer(&self, owner: u32) -> Option<u32> {
        self.counted.get(&owner).map(|text| text.buffer)
    }
    /// Explicit replacement-backend pointer/storage event, not an inferred
    /// native allocator. A returned old buffer remains live until completion.
    pub fn replace_counted_storage(&mut self, owner: u32, text: CountedBinding) -> Result<()> {
        self.validate_owned()?;
        ensure!(
            self.counted.contains_key(&owner),
            "replacement CPState owner absent"
        );
        self.allocate_buffer(owner, text)?;
        if !self.pending() {
            let live: BTreeSet<_> = self.counted.values().map(|c| c.buffer).collect();
            self.buffers.retain(|id, _| live.contains(id));
        }
        Ok(())
    }
    pub fn request(&self) -> Option<&native::Request> {
        self.call.as_ref()?.pending_request.as_ref()
    }
    pub fn reply_ready(&self) -> bool {
        self.call.as_ref().is_some_and(|c| c.ready_reply.is_some())
    }
    fn expect_service(&self, request: &native::Request) -> Result<()> {
        ensure!(
            self.request() == Some(request) && !self.reply_ready(),
            "family window service is stale or already executed"
        );
        Ok(())
    }
    fn retain_reply(&mut self, request: &native::Request, reply: native::Reply) -> Result<()> {
        self.expect_service(request)?;
        let call = self.call.as_mut().unwrap();
        call.native
            .clone()
            .acknowledge(request, reply.clone())
            .map_err(anyhow::Error::msg)?;
        call.ready_reply = Some(reply);
        Ok(())
    }
    pub fn validate(&self, session: &Session) -> Result<()> {
        self.validate_owned()?;
        if self.pending() {
            ensure!(
                self.call.as_ref().unwrap().tick == session.simulation.vm.tick,
                "family window crossed lot tick"
            );
            session.family_window_exclusive()?;
        }
        Ok(())
    }
    fn validate_owned(&self) -> Result<()> {
        ensure!(
            self.fields.owner != 0 && self.next_buffer != 0,
            "family window identity invalid"
        );
        ensure!(
            !self.counted.contains_key(&self.fields.owner)
                && !self.labels.contains_key(&self.fields.owner)
                && self.counted.keys().all(|id| !self.labels.contains_key(id)),
            "logical ViewControl/CPState/label owner classes alias"
        );
        for (&id, text) in &self.counted {
            let buffer = self
                .buffers
                .get(&text.buffer)
                .context("counted buffer owner absent")?;
            ensure!(
                id != 0 && buffer.state == id && text.length as usize == buffer.bytes.len(),
                "counted buffer identity/length differs"
            );
            if let Some(capacity) = text.capacity {
                ensure!(
                    buffer.bytes.len() < usize::from(capacity),
                    "counted storage exceeds supplied capacity"
                );
            }
        }
        for (&id, buffer) in &self.buffers {
            ensure!(
                id != 0 && id < self.next_buffer && self.counted.contains_key(&buffer.state),
                "forged CString buffer topology"
            );
            string(&buffer.bytes)?;
        }
        for (&id, bytes) in &self.labels {
            ensure!(id != 0, "null label owner");
            string(bytes)?;
        }
        ensure!(
            self.created <= 2 && self.destroyed.len() <= 2,
            "family window temporary extent invalid"
        );
        let expected: BTreeSet<_> = (1..=self.created)
            .filter(|id| !self.destroyed.contains(id))
            .collect();
        ensure!(
            self.temporaries.keys().copied().collect::<BTreeSet<_>>() == expected,
            "temporary owner missing or duplicated"
        );
        ensure!(
            self.destroyed.is_empty()
                || (self.created == 1 && self.destroyed == [1])
                || (self.created == 2 && (self.destroyed == [2] || self.destroyed == [2, 1])),
            "temporary destruction order differs"
        );
        for bytes in self.temporaries.values() {
            string(bytes)?;
        }
        if let Some(call) = &self.call {
            call.native.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                call.serial != 0
                    && call.serial == self.serial
                    && call.native.owner() == self.fields.owner
                    && call.native.raw_argument() == call.raw_argument,
                "family window caller identity differs"
            );
            ensure!(
                call.native.request() == call.pending_request,
                "family window pending service differs"
            );
            if let Some(reply) = &call.ready_reply {
                let request = call
                    .pending_request
                    .as_ref()
                    .context("ready reply lost request")?;
                call.native
                    .clone()
                    .acknowledge(request, reply.clone())
                    .map_err(anyhow::Error::msg)?;
            }
            if call.native.complete() {
                ensure!(
                    call.pending_request.is_none()
                        && call.ready_reply.is_none()
                        && self.temporaries.is_empty(),
                    "completed callback retained services/temporaries"
                );
                if call.finalized {
                    ensure!(
                        self.buffers.len() == self.counted.len(),
                        "finalized callback retained stale buffers"
                    );
                }
            }
            ensure!(
                !call.finalized || call.native.complete(),
                "window finalized before native return"
            );
        } else {
            ensure!(
                self.serial == 0
                    && self.temporaries.is_empty()
                    && self.created == 0
                    && self.destroyed.is_empty(),
                "family window continuation absent"
            );
            ensure!(
                self.buffers.len() == self.counted.len(),
                "unstarted window retained stale buffers"
            );
        }
        Ok(())
    }
    fn begin(&mut self, tick: u64, raw_argument: u32) -> Result<()> {
        self.validate_owned()?;
        ensure!(!self.pending(), "family window already owns a callback");
        let serial = self
            .serial
            .checked_add(1)
            .context("family window serial overflow")?;
        self.created = 0;
        self.destroyed.clear();
        self.serial = serial;
        self.call = Some(Execution {
            tick,
            serial,
            raw_argument,
            native: native::Call::new(self.fields.owner, raw_argument),
            pending_request: None,
            ready_reply: None,
            finalized: false,
        });
        Ok(())
    }
    fn step(&mut self) -> Result<Progress> {
        self.validate_owned()?;
        let call = self
            .call
            .as_mut()
            .context("family window callback absent")?;
        if call.native.complete() {
            ensure!(
                self.temporaries.is_empty(),
                "returned callback retained temporaries"
            );
            let live: BTreeSet<_> = self.counted.values().map(|c| c.buffer).collect();
            self.buffers.retain(|id, _| live.contains(id));
            call.finalized = true;
            return Ok(Progress::Complete);
        }
        if let Some(reply) = call.ready_reply.as_ref() {
            let request = call
                .pending_request
                .as_ref()
                .context("ready callback request absent")?;
            call.native
                .acknowledge(request, reply.clone())
                .map_err(anyhow::Error::msg)?;
            call.ready_reply = None;
            call.pending_request = None;
            return Ok(Progress::Pending);
        }
        if let Some(request) = &call.pending_request {
            return Ok(Progress::Service(request.clone()));
        }
        match call
            .native
            .advance(Some(&self.fields))
            .map_err(anyhow::Error::msg)?
        {
            Some(request) => {
                call.pending_request = Some(request.clone());
                Ok(Progress::Service(request))
            }
            None => {
                ensure!(
                    call.native.complete() && self.temporaries.is_empty(),
                    "callback completed before temporary destruction"
                );
                // Retain a cold cut after native return, before logical buffer pruning.
                Ok(Progress::Pending)
            }
        }
    }
    fn allocate_buffer(&mut self, state: u32, text: CountedBinding) -> Result<()> {
        string(&text.bytes)?;
        ensure!(state != 0, "CPState counted-text identity null");
        if let Some(capacity) = text.capacity {
            ensure!(
                text.bytes.len() < usize::from(capacity),
                "supplied text exceeds explicit capacity"
            );
        }
        let id = self.next_buffer;
        let next = id.checked_add(1).context("CString identity overflow")?;
        self.buffers.insert(
            id,
            Buffer {
                state,
                bytes: text.bytes.clone(),
            },
        );
        self.counted.insert(
            state,
            Counted {
                length: text.bytes.len() as u32,
                buffer: id,
                capacity: text.capacity,
                profile: text.profile,
            },
        );
        self.next_buffer = next;
        Ok(())
    }
    fn store(&mut self, state: u32, bytes: Vec<u8>) -> Result<()> {
        string(&bytes)?;
        let text = self
            .counted
            .get_mut(&state)
            .context("CPState counted-text dereference unavailable")?;
        ensure!(
            text.profile == Some(StorageProfile::CompleteCollection191Inline256Observed20261025)
                && text.capacity == Some(256),
            "CString allocation/capacity path unsupported or unknown"
        );
        ensure!(
            bytes.len() < 256,
            "CString growth requires an unmodeled allocator"
        );
        let buffer = self
            .buffers
            .get_mut(&text.buffer)
            .context("counted buffer owner absent")?;
        ensure!(buffer.state == state, "counted buffer identity differs");
        buffer.bytes = bytes;
        text.length = buffer.bytes.len() as u32;
        Ok(())
    }
    fn text_service(&mut self, request: &native::Request) -> Result<native::Reply> {
        use native::{
            NumericReply as P, NumericRequest as R, Reply, WindowReply as W, WindowRequest as Q,
        };
        Ok(match request {
            native::Request::Numeric(R::Store { state, bytes }) => {
                self.store(*state, bytes.clone())?;
                Reply::Numeric(P::Done)
            }
            native::Request::Numeric(R::CString { state }) => {
                let text = self
                    .counted
                    .get(state)
                    .context("CPState CString dereference unavailable")?;
                Reply::Numeric(P::Text(native::CountText {
                    state: *state,
                    buffer: text.buffer,
                }))
            }
            native::Request::Window(Q::Temporary { source }) => {
                let buffer = self
                    .buffers
                    .get(&source.buffer)
                    .context("CString source buffer absent")?;
                ensure!(buffer.state == source.state, "CString buffer changed owner");
                let id = self
                    .created
                    .checked_add(1)
                    .context("temporary identity overflow")?;
                ensure!(
                    id <= 2 && self.destroyed.is_empty(),
                    "temporary constructor order differs"
                );
                let bytes = buffer.bytes.clone();
                self.temporaries.insert(id, bytes.clone());
                self.created = id;
                Reply::Window(W::Bytes(bytes))
            }
            native::Request::Window(Q::Equal { left, widget }) => {
                ensure!(
                    self.temporaries.get(&1) == Some(left),
                    "comparison lost first temporary"
                );
                let right = self
                    .labels
                    .get(widget)
                    .context("label comparison dereference unavailable")?;
                Reply::Window(W::Equal(left == right))
            }
            native::Request::Window(Q::SetText { widget, bytes }) => {
                ensure!(
                    self.temporaries.get(&2) == Some(bytes),
                    "SetText lost second temporary"
                );
                *self
                    .labels
                    .get_mut(widget)
                    .context("SetText label dereference unavailable")? = bytes.clone();
                Reply::Window(W::Done)
            }
            native::Request::Window(Q::Destroy { temporary }) => {
                ensure!(
                    (*temporary == 2 && self.created == 2 && self.destroyed.is_empty())
                        || (*temporary == 1
                            && ((self.created == 1 && self.destroyed.is_empty())
                                || (self.created == 2 && self.destroyed == [2]))),
                    "temporary destructor order differs"
                );
                self.temporaries
                    .remove(temporary)
                    .context("temporary destructor owner absent")?;
                self.destroyed.push(*temporary);
                Reply::Window(W::Done)
            }
            _ => bail!("not a family window text service"),
        })
    }
}

fn family_handle(id: i16) -> u32 {
    0x10000 | u32::from(id as u16)
}
fn person_handle(id: i16) -> u32 {
    0x20000 | u32::from(id as u16)
}
fn family_id(handle: u32) -> Result<i16> {
    ensure!(handle >> 16 == 1, "family logical handle invalid");
    Ok(handle as u16 as i16)
}
fn person_id(handle: u32) -> Result<i16> {
    ensure!(handle >> 16 == 2, "person logical handle invalid");
    Ok(handle as u16 as i16)
}
fn lot_query(world: &World) -> Result<bool> {
    let lot = *world
        .globals
        .get(&10)
        .context("family text lot number absent")?;
    if lot == 0 {
        return Ok(true);
    }
    Ok(world
        .person
        .as_ref()
        .context("family text zoning owner absent")?
        .lot_zoning
        .get(&lot)
        .copied()
        .unwrap_or(0)
        == 1)
}
fn person(world: &World, index: u32) -> Result<Option<u32>> {
    let guid = sims_vm::interface::person_selection::cache_guid(&world.image_cache, index);
    if guid == 0 {
        return Ok(None);
    }
    let people = world
        .person
        .as_ref()
        .context("family text person registry absent")?;
    for id in &people.person_order {
        let definition = world
            .objects
            .get(id)
            .and_then(|o| o.definition.as_ref())
            .context("family text ordered person definition absent")?;
        if definition.guid == guid {
            return Ok(Some(person_handle(*id)));
        }
    }
    Ok(None)
}
fn family(world: &World, neighbor: i16) -> Result<Option<u32>> {
    let people = world
        .person
        .as_ref()
        .context("family text neighborhood absent")?;
    for id in people
        .family_order
        .as_ref()
        .context("family text family-header order absent")?
    {
        ensure!(
            people.families.contains_key(id),
            "family text ordered family header absent"
        );
        if *id == neighbor {
            return Ok(Some(family_handle(*id)));
        }
    }
    Ok(None)
}
fn friends(world: &mut World, handle: u32) -> Result<i32> {
    world
        .person
        .as_mut()
        .context("family cache neighborhood absent")?
        .resolve_family_friend_count(family_id(handle)?, world.autonomy_constants.as_ref())
        .map_err(anyhow::Error::msg)
}
fn decimal(value: i32) -> Vec<u8> {
    value.to_string().into_bytes()
}
fn application_family(value: ApplicationFamily) -> Result<Option<u32>> {
    Ok(match value {
        ApplicationFamily::Unknown => bail!("application family owner unknown"),
        ApplicationFamily::KnownNull => None,
        ApplicationFamily::Family(id) => Some(family_handle(id)),
    })
}

impl Session {
    /// Deliberately excludes this child from its own exclusivity validation.
    fn family_window_exclusive(&self) -> Result<()> {
        ensure!(
            !self.transport_pending()
                && !self
                    .host
                    .presentation
                    .interface_update
                    .as_ref()
                    .is_some_and(super::interface::State::pending)
                && !self.player_input_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.clock_pending()
                && !self.person_draw_preparation_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.view_refresh_pending()
                && !self.house_notification_pending()
                && !self.host.presentation.awaiting_house_open
                && !self.host.ordinary.updates_pending()
                && !self.simulation.selection_work_pending()
                && self.simulation.vm.scheduled_object_visit.is_none()
                && self.simulation.vm.lot_rng_scope.is_none()
                && self.simulation.visit.is_none()
                && !self.simulation.vm.constructing_dialog(),
            "family window overlaps another synchronous caller"
        );
        Ok(())
    }
    pub fn family_window_pending(&self) -> bool {
        self.host
            .presentation
            .family_window
            .as_ref()
            .is_some_and(State::pending)
    }
    pub fn bind_family_window(&mut self, binding: Binding) -> Result<()> {
        self.family_window_exclusive()?;
        ensure!(
            !self.family_window_pending(),
            "pending family window cannot be replaced"
        );
        let state = State::bind(binding)?;
        self.host.presentation.family_window = Some(state);
        Ok(())
    }
    /// Explicit observed child invocation only. No load/frame/render calls it.
    pub fn begin_family_window_update(&mut self, raw_argument: u32) -> Result<()> {
        self.family_window_exclusive()?;
        self.host
            .presentation
            .family_window
            .as_mut()
            .context("family window unbound")?
            .begin(self.simulation.vm.tick, raw_argument)
    }
    pub fn advance_family_window_update(&mut self, budget: usize) -> Result<Progress> {
        self.host
            .presentation
            .family_window
            .as_ref()
            .context("family window unbound")?
            .validate(self)?;
        for _ in 0..budget {
            let progress = self
                .host
                .presentation
                .family_window
                .as_mut()
                .unwrap()
                .step()?;
            if progress != Progress::Pending {
                return Ok(progress);
            }
        }
        Ok(Progress::Pending)
    }
    /// Execute once, retain the ready reply, then acknowledge in a later step.
    pub fn service_family_window_request(&mut self, request: &native::Request) -> Result<()> {
        let state = self
            .host
            .presentation
            .family_window
            .as_ref()
            .context("family window unbound")?;
        state.validate(self)?;
        state.expect_service(request)?;
        use native::{NumericReply as P, NumericRequest as R, Reply};
        let reply = match request {
            native::Request::Numeric(r) => match r {
                R::LotQuery => Reply::Numeric(P::Boolean(lot_query(&self.simulation.vm.world)?)),
                R::ApplicationFamily => {
                    Reply::Numeric(P::Owner(application_family(state.application_family)?))
                }
                R::SelectedPerson => {
                    Reply::Numeric(P::Number(self.selected_portrait_index()? as u32))
                }
                R::Person { index } => {
                    Reply::Numeric(P::Owner(person(&self.simulation.vm.world, *index)?))
                }
                R::NeighborWord { person } => Reply::Numeric(P::Word(
                    *self
                        .simulation
                        .vm
                        .world
                        .objects
                        .get(&person_id(*person)?)
                        .and_then(|o| o.person.as_ref())
                        .and_then(|p| p.get(&61))
                        .context("family text live person family word absent")?
                        as u16,
                )),
                R::Family { neighbor } => {
                    Reply::Numeric(P::Owner(family(&self.simulation.vm.world, *neighbor)?))
                }
                R::Friends { family } => {
                    let value = friends(&mut self.simulation.vm.world, *family)?;
                    Reply::Numeric(P::Count(value))
                }
                R::Format { value } => Reply::Numeric(P::Bytes(decimal(*value))),
                R::Store { .. } | R::CString { .. } => self
                    .host
                    .presentation
                    .family_window
                    .as_mut()
                    .unwrap()
                    .text_service(request)?,
            },
            native::Request::Window(_) => self
                .host
                .presentation
                .family_window
                .as_mut()
                .unwrap()
                .text_service(request)?,
        };
        self.host
            .presentation
            .family_window
            .as_mut()
            .unwrap()
            .retain_reply(request, reply)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "window_update/tests.rs"]
mod tests;
