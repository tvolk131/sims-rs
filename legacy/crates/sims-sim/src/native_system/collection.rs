//! Owning collection/list prefix at factory5d5879, stopping BEFORE5d9339.
//! Only explicit source operands and normal section returns are modeled. No
//! resource import, collection/factory return, global publication or alpha gate
//! is produced. The host owns live-heap provenance and monotonic synchronization.
use super::factory;
use crate::{
    native_application::{self, InitializedSection, Write},
    native_world_children::Owner,
    saved_matrix::{Allocated, MAX_ALLOCATION_BYTES, Stop},
};
use std::sync::Arc;

pub mod event;

#[cfg(test)]
mod tests;

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}
fn unavailable(message: &str) -> Stop {
    Stop::Unavailable(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frames {
    pub collection: u64,
    pub list: u64,
    pub section_base: u64,
    pub resource_argument: u64,
}
#[derive(Debug)]
pub struct Incoming {
    pub factory: factory::Checkpoint,
    pub frames: Frames,
}
#[derive(Debug)]
pub struct BeginError {
    pub stop: Stop,
    pub incoming: Incoming,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionSite {
    Collection,
    List,
    Resource,
}
impl SectionSite {
    fn index(self) -> usize {
        match self {
            Self::Collection => 0,
            Self::List => 1,
            Self::Resource => 2,
        }
    }
    fn interface(self) -> usize {
        match self {
            Self::Collection => 132,
            Self::List => 176,
            Self::Resource => 220,
        }
    }
}
const SITES: [SectionSite; 3] = [
    SectionSite::Collection,
    SectionSite::List,
    SectionSite::Resource,
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    BeforeResource,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    AllocateList,
    InitializeSection(SectionSite),
    Resource {
        instruction: u32,
        import_slot: u32,
        arguments: [u32; 4],
    },
}
pub trait Host {
    /// Validate the issuing factory heap and all current complete allocations;
    /// reject stale source ordinals instead of resetting newer live memory.
    fn synchronize_checkpoint(&mut self, cut: &Checkpoint) -> Result<(), Stop>;
    /// An effect followed by Err must remain host-owned and service the SAME
    /// sealed request on retry, without reissuing the allocation/initialization.
    fn allocate_list(&mut self, request: &ListRequest) -> Result<Allocated, Stop>;
    fn initialize_section(&mut self, call: &SectionCall) -> Result<InitializedSection, Stop>;
}

fn end(owner: &Owner) -> Result<u64, Stop> {
    if owner.token == 0 || owner.data.is_empty() || owner.data.len() > MAX_ALLOCATION_BYTES {
        return Err(unsupported("collection needs complete bounded owners"));
    }
    owner
        .token
        .checked_add(owner.data.len() as u64)
        .filter(|n| *n <= 1_u64 << 32)
        .ok_or_else(|| unsupported("collection owner exceeds native width"))
}
fn operand(caller: &Owner, address: u64, bytes: u64) -> Result<(u64, u64), Stop> {
    let last = address
        .checked_add(bytes)
        .filter(|n| *n <= 1_u64 << 32)
        .ok_or_else(|| unsupported("collection operand exceeds native width"))?;
    if address < caller.token || last > end(caller)? {
        return Err(unsupported(
            "collection operands need ONE complete caller owner",
        ));
    }
    Ok((address, last))
}
fn relative(ebp: u64, offset: i32) -> Result<u64, Stop> {
    if ebp == 0 || ebp > u64::from(u32::MAX) {
        return Err(unsupported("collection EBP exceeds native width"));
    }
    ebp.checked_add_signed(i64::from(offset))
        .ok_or_else(|| unsupported("collection frame operand wraps"))
}
fn aliases(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}
fn reserve_return_budget(total: usize) -> Result<(), Stop> {
    if total
        .checked_add(24 + 3 * 32)
        .is_none_or(|n| n > MAX_ALLOCATION_BYTES)
    {
        return Err(unsupported(
            "collection has no budget for its bounded actual returns",
        ));
    }
    Ok(())
}

fn retained(
    factory: &factory::Checkpoint,
    list: Option<&Owner>,
    sections: &[Option<InitializedSection>; 3],
) -> Result<usize, Stop> {
    // Embedded section regions are views of parent, never independent owners.
    let owners = [
        Some(factory.parent()),
        Some(factory.caller()),
        factory.tree(factory::TreeSite::Refcount),
        factory.tree(factory::TreeSite::Derived),
        factory
            .base72()
            .and_then(|p| p.section().debug_info.as_ref()),
        list,
        sections[0].as_ref().and_then(|s| s.debug_info.as_ref()),
        sections[1].as_ref().and_then(|s| s.debug_info.as_ref()),
        sections[2].as_ref().and_then(|s| s.debug_info.as_ref()),
    ];
    let mut total = 0_usize;
    for (at, owner) in owners.iter().enumerate() {
        if let Some(owner) = owner {
            let span = (owner.token, end(owner)?);
            total = total
                .checked_add(owner.data.len())
                .filter(|n| *n <= MAX_ALLOCATION_BYTES)
                .ok_or_else(|| unsupported("collection retained allocation budget exceeded"))?;
            for other in owners[..at].iter().flatten() {
                if aliases(span, (other.token, end(other)?)) {
                    return Err(unsupported(
                        "collection complete retained allocations alias",
                    ));
                }
            }
        }
    }
    Ok(total)
}
fn admit(factory: &factory::Checkpoint, frames: Frames) -> Result<(), Stop> {
    factory.validate()?;
    if factory.parent().data.len() != 1240
        || factory
            .writes()
            .last()
            .is_none_or(|w| w.instruction != 0x5d5875 || w.after != [2])
        || factory.tree(factory::TreeSite::Refcount).is_none()
        || factory.tree(factory::TreeSite::Derived).is_none()
        || factory.base72().is_none()
        || factory.base72_call().is_none()
    {
        return Err(unsupported(
            "collection requires the exact factory5d5879 frontier after5d5875",
        ));
    }
    let caller = factory.caller();
    let new = [
        operand(caller, relative(frames.collection, -20)?, 4)?,
        operand(caller, relative(frames.collection, -4)?, 4)?,
        operand(caller, relative(frames.list, -16)?, 4)?,
        operand(caller, relative(frames.list, -4)?, 4)?,
        operand(caller, relative(frames.section_base, -16)?, 4)?,
        operand(caller, relative(frames.section_base, -4)?, 4)?,
        operand(caller, frames.resource_argument, 4)?,
    ];
    let f = factory.request().incoming().frames;
    // Exact persistent source operands, not invented full machine frames.
    let old = [
        (f.factory, -16, 4),
        (f.factory, -4, 4),
        (f.derived, -16, 4),
        (f.base, -16, 4),
        (f.base, -4, 4),
        (f.refcount, -2, 1),
        (f.child, -20, 4),
        (f.child, -14, 1),
        (f.child, -4, 4),
        (f.base72, -16, 4),
        (f.base72, -4, 4),
        (f.base72, 8, 4),
    ];
    for (at, range) in new.iter().enumerate() {
        if new[..at].iter().any(|other| aliases(*range, *other)) {
            return Err(unsupported("collection new caller operands alias"));
        }
        for (ebp, offset, bytes) in old {
            if aliases(*range, operand(caller, relative(ebp, offset)?, bytes)?) {
                return Err(unsupported(
                    "collection caller operands alias live factory operands",
                ));
            }
        }
    }
    // Reserve the full logical ownership budget BEFORE any new external call,
    // including the possible real list and three real32-byte debug allocations.
    reserve_return_budget(retained(factory, None, &[None, None, None])?)
}

#[derive(Debug)]
struct Invocation {
    factory: factory::Checkpoint,
    frames: Frames,
}
/// No public adoption or deserialization; cloning retains the invocation.
#[derive(Debug, Clone)]
pub struct Request {
    invocation: Arc<Invocation>,
}
impl Request {
    pub fn factory(&self) -> &factory::Checkpoint {
        &self.invocation.factory
    }
    pub fn frames(&self) -> Frames {
        self.invocation.frames
    }
    pub fn same_call(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.invocation, &other.invocation)
    }
    pub fn validate(&self) -> Result<(), Stop> {
        admit(self.factory(), self.frames())
    }
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    CollectionSelf,
    CollectionEh,
    ListSelf,
    ListEh,
    BaseSelf,
    BaseEh,
    Argument,
}
impl Slot {
    fn offset(self, request: &Request) -> Result<usize, Stop> {
        let f = request.frames();
        let address = match self {
            Self::CollectionSelf => relative(f.collection, -20)?,
            Self::CollectionEh => relative(f.collection, -4)?,
            Self::ListSelf => relative(f.list, -16)?,
            Self::ListEh => relative(f.list, -4)?,
            Self::BaseSelf => relative(f.section_base, -16)?,
            Self::BaseEh => relative(f.section_base, -4)?,
            Self::Argument => f.resource_argument,
        };
        operand(request.factory().caller(), address, 4)?;
        usize::try_from(address - request.factory().caller().token)
            .map_err(|_| unsupported("collection caller offset exceeds host width"))
    }
}
#[derive(Debug, Clone, Copy)]
enum Place {
    Parent(usize),
    Caller(Slot),
    List(usize),
}
#[derive(Debug, Clone, Copy)]
enum Value {
    Word(u32),
    ParentAddress(u32),
    ListAddress,
    ZeroSection,
    Section(SectionSite),
    Getter,
    Argument,
}
#[derive(Debug, Clone, Copy)]
struct Store {
    place: Place,
    value: Value,
    pc: u32,
    byte: bool,
}
impl Store {
    const fn parent(offset: usize, value: u32, pc: u32) -> Op {
        Op::Store(Self {
            place: Place::Parent(offset),
            value: Value::Word(value),
            pc,
            byte: false,
        })
    }
    fn width(self) -> usize {
        match self.value {
            Value::ZeroSection | Value::Section(_) => 24,
            _ if self.byte => 1,
            _ => 4,
        }
    }
    fn child_return(self) -> bool {
        matches!(self.value, Value::ZeroSection | Value::Section(_))
    }
    fn location(self, request: &Request, list: Option<&Owner>) -> Result<(u64, usize), Stop> {
        Ok(match self.place {
            Place::Parent(offset) => (request.factory().parent().token, offset),
            Place::Caller(slot) => (request.factory().caller().token, slot.offset(request)?),
            Place::List(offset) => (
                list.ok_or_else(|| unsupported("collection list return is absent"))?
                    .token,
                offset,
            ),
        })
    }
    fn bytes(
        self,
        parent: &Owner,
        caller: &Owner,
        list: Option<&Owner>,
        request: &Request,
        sections: &[Option<InitializedSection>; 3],
    ) -> Result<([u8; 24], u32), Stop> {
        let mut result = [0; 24];
        let mut pc = self.pc;
        let value = match self.value {
            Value::Word(value) => value,
            Value::ParentAddress(offset) => (parent.token + u64::from(offset)) as u32,
            Value::ListAddress => {
                list.ok_or_else(|| unsupported("collection lacks actual list pointer"))?
                    .token as u32
            }
            Value::ZeroSection => return Ok((result, pc)),
            Value::Section(site) => {
                let section = sections[site.index()]
                    .as_ref()
                    .ok_or_else(|| unsupported("collection lacks actual section return"))?;
                result.copy_from_slice(&section.region.data);
                return Ok((result, pc));
            }
            Value::Getter => {
                //5d948b calls the constructed interface's status getter (+28).
                //5d948e..5d949b normalizes that actual status, not a resource result.
                let status = word(&parent.data[248..252]);
                if status == 0 {
                    pc = 0x5d949b;
                    0
                } else {
                    1
                }
            }
            Value::Argument => {
                let at = Slot::Argument.offset(request)?;
                word(&caller.data[at..at + 4])
            }
        };
        result[..4].copy_from_slice(&value.to_le_bytes());
        Ok((result, pc))
    }
}
#[derive(Debug, Clone, Copy)]
enum Op {
    Store(Store),
    Allocate,
}
const fn caller(slot: Slot, value: Value, pc: u32, byte: bool) -> Op {
    Op::Store(Store {
        place: Place::Caller(slot),
        value,
        pc,
        byte,
    })
}
const fn section_zero(offset: usize) -> Op {
    Op::Store(Store {
        place: Place::Parent(offset),
        value: Value::ZeroSection,
        pc: 0x5d91b3,
        byte: false,
    })
}
const fn section_return(offset: usize, site: SectionSite) -> Op {
    Op::Store(Store {
        place: Place::Parent(offset),
        value: Value::Section(site),
        pc: 0x5d91bc,
        byte: false,
    })
}
const OPS: &[Op] = &[
    caller(
        Slot::CollectionSelf,
        Value::ParentAddress(128),
        0x5d5d05,
        false,
    ),
    Store::parent(128, 0x673884, 0x5d5d0b),
    Store::parent(160, 0, 0x5d91a7),
    Store::parent(164, 0, 0x5d91aa),
    Store::parent(132, 0x673ab8, 0x5d91ad),
    section_zero(136),
    section_return(136, SectionSite::Collection),
    Store::parent(160, 1, 0x5d91c2),
    caller(Slot::CollectionEh, Value::Word(0), 0x5d5d16, false),
    caller(Slot::ListSelf, Value::ParentAddress(168), 0x6017bf, false),
    Store::parent(168, 0x676a00, 0x6017c2),
    Op::Allocate,
    Op::Store(Store {
        place: Place::Parent(172),
        value: Value::ListAddress,
        pc: 0x6017cd,
        byte: false,
    }),
    Op::Store(Store {
        place: Place::List(0),
        value: Value::ListAddress,
        pc: 0x6017d0,
        byte: false,
    }),
    Op::Store(Store {
        place: Place::List(4),
        value: Value::ListAddress,
        pc: 0x6017d6,
        byte: false,
    }),
    caller(Slot::ListEh, Value::Word(0), 0x6017d9, false),
    Store::parent(204, 0, 0x5d91a7),
    Store::parent(208, 0, 0x5d91aa),
    Store::parent(176, 0x673ab8, 0x5d91ad),
    section_zero(180),
    section_return(180, SectionSite::List),
    Store::parent(204, 1, 0x5d91c2),
    caller(Slot::Argument, Value::Word(0), 0x6017e5, false),
    caller(Slot::ListEh, Value::Word(1), 0x6017ea, true),
    Store::parent(212, 0x673b00, 0x5d9310),
    caller(Slot::BaseSelf, Value::ParentAddress(216), 0x5d9470, false),
    Store::parent(248, 0, 0x5d91a7),
    Store::parent(252, 0, 0x5d91aa),
    Store::parent(220, 0x673ab8, 0x5d91ad),
    section_zero(224),
    section_return(224, SectionSite::Resource),
    Store::parent(248, 1, 0x5d91c2),
    caller(Slot::BaseEh, Value::Word(0), 0x5d947f, false),
    Store::parent(216, 0x673858, 0x5d9485),
    Op::Store(Store {
        place: Place::Parent(256),
        value: Value::Getter,
        pc: 0x5d9492,
        byte: false,
    }),
    Store::parent(216, 0x673af8, 0x5d9321),
    Store::parent(212, 0x673adc, 0x5d9329),
    Store::parent(268, 0, 0x5d9333),
    Op::Store(Store {
        place: Place::Parent(264),
        value: Value::Argument,
        pc: 0x5d9336,
        byte: false,
    }),
];
fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("admitted DWORD"))
}
fn list_ordinal() -> usize {
    OPS.iter()
        .position(|op| matches!(op, Op::Allocate))
        .unwrap()
}
fn section_ordinal(site: SectionSite) -> usize {
    OPS.iter()
        .position(
            |op| matches!(op, Op::Store(Store { value: Value::Section(s), .. }) if *s == site),
        )
        .unwrap()
}
fn write_count(position: usize) -> usize {
    OPS[..position]
        .iter()
        .filter(|op| matches!(op, Op::Store(_)))
        .count()
}
fn storage(bytes: usize) -> Result<Vec<u8>, Stop> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(bytes)
        .map_err(|_| unavailable("collection bounded snapshot/history quota unavailable"))?;
    result.resize(bytes, 0);
    Ok(result)
}
fn snapshot(owner: &Owner) -> Result<Owner, Stop> {
    let mut data = storage(owner.data.len())?;
    data.copy_from_slice(&owner.data);
    Ok(Owner {
        token: owner.token,
        data,
    })
}
fn optional_snapshot(owner: Option<&Owner>) -> Result<Option<Owner>, Stop> {
    owner.map(snapshot).transpose()
}
fn section_snapshot(section: &InitializedSection) -> Result<InitializedSection, Stop> {
    Ok(InitializedSection {
        region: snapshot(&section.region)?,
        capability: section.capability,
        debug_info: optional_snapshot(section.debug_info.as_ref())?,
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListReceipt {
    token: u64,
    bytes: [u8; 24],
}
impl ListReceipt {
    fn capture(owner: &Owner) -> Option<Self> {
        Some(Self {
            token: owner.token,
            bytes: owner.data.as_slice().try_into().ok()?,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SectionReceipt {
    token: u64,
    bytes: [u8; 24],
    capability: u64,
    debug: Option<(u64, [u8; 32])>,
}
impl SectionReceipt {
    fn capture(section: &InitializedSection) -> Option<Self> {
        Some(Self {
            token: section.region.token,
            bytes: section.region.data.as_slice().try_into().ok()?,
            capability: section.capability,
            debug: match &section.debug_info {
                Some(owner) => Some((owner.token, owner.data.as_slice().try_into().ok()?)),
                None => None,
            },
        })
    }
}

/// Complete source snapshots and actual replies, including a staged reply
/// before its source store. A malformed staged reply makes validation fail;
/// it stays owned by Construction and cannot be replaced by another host call.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    request: Request,
    parent: Owner,
    caller: Owner,
    list: Option<Owner>,
    initial_list: Option<Owner>,
    list_returned: bool,
    list_receipt: Option<ListReceipt>,
    sections: [Option<InitializedSection>; 3],
    receipts: [Option<SectionReceipt>; 3],
    list_call: Option<Arc<ListRequest>>,
    section_calls: [Option<Arc<SectionCall>>; 3],
    history: Vec<Write>,
    position: usize,
}
impl Checkpoint {
    pub fn request(&self) -> &Request {
        &self.request
    }
    pub fn parent(&self) -> &Owner {
        &self.parent
    }
    pub fn caller_frame(&self) -> &Owner {
        &self.caller
    }
    pub fn list(&self) -> Option<&Owner> {
        self.list.as_ref()
    }
    pub fn initial_list(&self) -> Option<&Owner> {
        self.initial_list.as_ref()
    }
    pub fn sections(&self) -> &[Option<InitializedSection>; 3] {
        &self.sections
    }
    pub fn writes(&self) -> &[Write] {
        &self.history[..write_count(self.position)]
    }
    pub fn source_ordinal(&self) -> usize {
        self.position
    }

    fn snapshot(&self) -> Result<Self, Stop> {
        let mut history = Vec::new();
        history
            .try_reserve_exact(self.history.len())
            .map_err(|_| unavailable("collection history snapshot quota unavailable"))?;
        for write in &self.history {
            let mut before = storage(write.before.len())?;
            let mut after = storage(write.after.len())?;
            before.copy_from_slice(&write.before);
            after.copy_from_slice(&write.after);
            history.push(Write {
                owner: write.owner,
                offset: write.offset,
                instruction: write.instruction,
                child_return: write.child_return,
                before,
                after,
            });
        }
        let mut sections = [None, None, None];
        for (to, from) in sections.iter_mut().zip(&self.sections) {
            *to = from.as_ref().map(section_snapshot).transpose()?;
        }
        Ok(Self {
            request: self.request.clone(),
            parent: snapshot(&self.parent)?,
            caller: snapshot(&self.caller)?,
            list: optional_snapshot(self.list.as_ref())?,
            initial_list: optional_snapshot(self.initial_list.as_ref())?,
            list_returned: self.list_returned,
            list_receipt: self.list_receipt,
            sections,
            receipts: self.receipts,
            list_call: self.list_call.clone(),
            section_calls: self.section_calls.clone(),
            history,
            position: self.position,
        })
    }

    fn call_origin(
        &self,
        cut: &Self,
        ordinal: usize,
        parent: &Owner,
        caller: &Owner,
        list: Option<&Owner>,
    ) -> Result<(), Stop> {
        cut.validate()?;
        if !cut.request.same_call(&self.request)
            || cut.position != ordinal
            || &cut.parent != parent
            || &cut.caller != caller
            || cut.list.as_ref() != list
            || cut.writes() != &self.history[..write_count(ordinal)]
            || cut.list_returned != (ordinal > list_ordinal())
            || cut.initial_list.as_ref()
                != (ordinal > list_ordinal())
                    .then_some(self.initial_list.as_ref())
                    .flatten()
        {
            return Err(unsupported(
                "collection call lost its exact issuing source images",
            ));
        }
        if ordinal > list_ordinal()
            && !matches!((&cut.list_call, &self.list_call), (Some(a), Some(b)) if a.same_call(b))
        {
            return Err(unsupported(
                "collection call lost its actual list invocation",
            ));
        }
        for site in SITES {
            let at = site.index();
            if ordinal > section_ordinal(site) {
                if cut.sections[at] != self.sections[at]
                    || cut.receipts[at] != self.receipts[at]
                    || !matches!((&cut.section_calls[at], &self.section_calls[at]), (Some(a), Some(b)) if a.same_call(b))
                {
                    return Err(unsupported(
                        "collection call lost a preceding section invocation/return",
                    ));
                }
            } else if cut.sections[at].is_some() || cut.section_calls[at].is_some() {
                return Err(unsupported(
                    "collection call snapshot retains a future section",
                ));
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.request.validate()?;
        if self.position > OPS.len() || self.history.len() != write_count(OPS.len()) {
            return Err(unsupported(
                "collection source ordinal/history is outside its bounded program",
            ));
        }
        let list_at = list_ordinal();
        if (self.position < list_at && (self.list_returned || self.list_call.is_some()))
            || (self.position > list_at && !self.list_returned)
            || self.list.is_some() != (self.position > list_at)
        {
            return Err(unsupported(
                "collection list return is outside its source position",
            ));
        }
        if self.list_returned {
            let owner = self.initial_list.as_ref().ok_or_else(|| {
                unavailable("collection list allocator returned null before source dereference")
            })?;
            let receipt = ListReceipt::capture(owner)
                .ok_or_else(|| unsupported("collection list return must contain actual24 bytes"))?;
            if self.list_receipt != Some(receipt) || self.list_call.is_none() {
                return Err(unsupported(
                    "collection list differs from its retained actual return",
                ));
            }
        } else if self.initial_list.is_some() || self.list_receipt.is_some() {
            return Err(unsupported("collection list preimage has no actual return"));
        }
        let base_capability = self
            .request
            .factory()
            .base72()
            .unwrap()
            .section()
            .capability;
        for site in SITES {
            let at = site.index();
            let ordinal = section_ordinal(site);
            if (self.position < ordinal
                && (self.sections[at].is_some() || self.section_calls[at].is_some()))
                || (self.position > ordinal && self.sections[at].is_none())
            {
                return Err(unsupported(
                    "collection section is outside its source position",
                ));
            }
            if let Some(section) = &self.sections[at] {
                let receipt = SectionReceipt::capture(section).ok_or_else(|| {
                    unsupported("collection retained section has malformed extents")
                })?;
                let call = self.section_calls[at]
                    .as_ref()
                    .ok_or_else(|| unsupported("collection section lost its issuing call"))?;
                call.validate()?;
                if self.receipts[at] != Some(receipt)
                    || call.site != site
                    || section.capability == base_capability
                    || self.sections[..at]
                        .iter()
                        .flatten()
                        .any(|s| s.capability == section.capability)
                {
                    return Err(unsupported(
                        "collection section receipt/capability was changed or swapped",
                    ));
                }
                section.validate(&call.section_request())?;
            } else if self.receipts[at].is_some() {
                return Err(unsupported(
                    "collection section receipt lost its owning return",
                ));
            }
        }
        retained(
            self.request.factory(),
            self.initial_list.as_ref(),
            &self.sections,
        )?;
        let mut parent = snapshot(self.request.factory().parent())?;
        let mut caller = snapshot(self.request.factory().caller())?;
        let mut list = None;
        let mut write_at = 0;
        for (ordinal, op) in OPS.iter().copied().enumerate() {
            match op {
                Op::Allocate => {
                    if let Some(call) = &self.list_call {
                        call.validate()?;
                        self.call_origin(
                            &call.checkpoint,
                            ordinal,
                            &parent,
                            &caller,
                            list.as_ref(),
                        )?;
                    } else if self.list_returned {
                        return Err(unsupported("collection allocation lost its sealed call"));
                    }
                    if ordinal < self.position {
                        list = optional_snapshot(self.initial_list.as_ref())?;
                    }
                }
                Op::Store(store) => {
                    if let Value::Section(site) = store.value {
                        if let Some(call) = &self.section_calls[site.index()] {
                            call.validate()?;
                            if call.site != site {
                                return Err(unsupported("collection section calls were swapped"));
                            }
                            self.call_origin(
                                &call.checkpoint,
                                ordinal,
                                &parent,
                                &caller,
                                list.as_ref(),
                            )?;
                        }
                    }
                    let receipt = &self.history[write_at];
                    if receipt.before.len() != store.width() || receipt.after.len() != store.width()
                    {
                        return Err(unsupported("collection history has another store width"));
                    }
                    if ordinal < self.position {
                        let (token, offset) = store.location(&self.request, list.as_ref())?;
                        let (bytes, pc) = store.bytes(
                            &parent,
                            &caller,
                            list.as_ref(),
                            &self.request,
                            &self.sections,
                        )?;
                        let owner = match store.place {
                            Place::Parent(_) => &mut parent,
                            Place::Caller(_) => &mut caller,
                            Place::List(_) => list
                                .as_mut()
                                .ok_or_else(|| unsupported("collection replay lost its list"))?,
                        };
                        let current = &mut owner.data[offset..offset + store.width()];
                        if receipt.owner != token
                            || receipt.offset != offset
                            || receipt.instruction != pc
                            || receipt.child_return != store.child_return()
                            || receipt.before.as_slice() != &*current
                            || receipt.after.as_slice() != &bytes[..store.width()]
                        {
                            return Err(unsupported(
                                "collection history differs from exact source replay",
                            ));
                        }
                        current.copy_from_slice(&bytes[..store.width()]);
                    } else if receipt.owner != 0
                        || receipt.offset != 0
                        || receipt.instruction != 0
                        || receipt.child_return
                        || receipt.before.iter().chain(&receipt.after).any(|b| *b != 0)
                    {
                        return Err(unsupported("collection future source history is populated"));
                    }
                    write_at += 1;
                }
            }
        }
        if parent != self.parent || caller != self.caller || list != self.list {
            return Err(unsupported(
                "collection complete images differ from sealed initial images/actual returns",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ListRequest {
    identity: Arc<()>,
    checkpoint: Checkpoint,
}
impl ListRequest {
    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }
    pub fn instruction(&self) -> u32 {
        0x6017c8
    }
    pub fn bytes(&self) -> u32 {
        24
    }
    pub fn same_call(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
            && self.checkpoint.request.same_call(&other.checkpoint.request)
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.checkpoint.validate()?;
        if self.checkpoint.position != list_ordinal()
            || self.checkpoint.list_call.is_some()
            || self.checkpoint.list_returned
        {
            return Err(unsupported(
                "collection list request is not its immutable pre-allocation cut",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct SectionCall {
    identity: Arc<()>,
    checkpoint: Checkpoint,
    site: SectionSite,
}
impl SectionCall {
    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }
    pub fn site(&self) -> SectionSite {
        self.site
    }
    pub fn section_request(&self) -> native_application::SectionRequest {
        let first = self.site.interface() + 4;
        native_application::SectionRequest {
            instruction: 0x5d91bc,
            import_slot: 0x666114,
            region: Owner {
                token: self.checkpoint.parent.token + first as u64,
                data: self.checkpoint.parent.data[first..first + 24].to_vec(),
            },
        }
    }
    pub fn same_call(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
            && self.site == other.site
            && self.checkpoint.request.same_call(&other.checkpoint.request)
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.checkpoint.validate()?;
        let at = self.site.index();
        if self.checkpoint.position != section_ordinal(self.site)
            || self.checkpoint.section_calls[at].is_some()
            || self.checkpoint.sections[at].is_some()
            || self.section_request().region.data != [0; 24]
        {
            return Err(unsupported(
                "collection section call is not its distinct pre-import source cut",
            ));
        }
        Ok(())
    }
}

/// Non-Clone/non-Deserialize owning cursor. All history buffers, complete
/// mutable images and the list image buffer are reserved before any host effect.
/// Each immutable call snapshot is also reserved before synchronization/effect.
#[derive(Debug)]
pub struct Construction {
    state: Checkpoint,
    list_buffer: Option<Owner>,
    stopped: bool,
    event: Option<event::Continuation>,
}
impl Construction {
    pub fn new(incoming: Incoming) -> Result<Self, BeginError> {
        let prepare = || -> Result<_, Stop> {
            admit(&incoming.factory, incoming.frames)?;
            let parent = snapshot(incoming.factory.parent())?;
            let caller = snapshot(incoming.factory.caller())?;
            let mut history = Vec::new();
            history
                .try_reserve_exact(write_count(OPS.len()))
                .map_err(|_| unavailable("collection history quota unavailable"))?;
            for op in OPS {
                if let Op::Store(store) = op {
                    history.push(Write {
                        owner: 0,
                        offset: 0,
                        instruction: 0,
                        child_return: false,
                        before: storage(store.width())?,
                        after: storage(store.width())?,
                    });
                }
            }
            Ok((
                parent,
                caller,
                history,
                Owner {
                    token: 0,
                    data: storage(24)?,
                },
            ))
        };
        let (parent, caller, history, list_buffer) = match prepare() {
            Ok(prepared) => prepared,
            Err(stop) => return Err(BeginError { stop, incoming }),
        };
        let request = Request {
            invocation: Arc::new(Invocation {
                factory: incoming.factory,
                frames: incoming.frames,
            }),
        };
        Ok(Self {
            state: Checkpoint {
                request,
                parent,
                caller,
                list: None,
                initial_list: None,
                list_returned: false,
                list_receipt: None,
                sections: [None, None, None],
                receipts: [None, None, None],
                list_call: None,
                section_calls: [None, None, None],
                history,
                position: 0,
            },
            list_buffer: Some(list_buffer),
            stopped: false,
            event: None,
        })
    }
    pub fn request(&self) -> &Request {
        &self.state.request
    }
    pub fn checkpoint(&self) -> Result<Checkpoint, Stop> {
        self.state.validate()?;
        self.state.snapshot()
    }
    pub fn pending_call(&self) -> Option<Call> {
        match OPS.get(self.state.position) {
            Some(Op::Allocate) => Some(Call::AllocateList),
            Some(Op::Store(Store {
                value: Value::Section(site),
                ..
            })) => Some(Call::InitializeSection(*site)),
            None if self.state.position == OPS.len() => {
                let at = Slot::Argument.offset(self.request()).ok()?;
                let argument = word(self.state.caller.data.get(at..at + 4)?);
                Some(Call::Resource {
                    instruction: 0x5d9339,
                    import_slot: 0x666238,
                    arguments: [0, argument, 0, 0],
                })
            }
            _ => None,
        }
    }
    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.state.validate()?;
        if self.stopped {
            return Err(unavailable(
                "collection prefix already stopped before resource666238",
            ));
        }
        if self.state.position == OPS.len() {
            host.synchronize_checkpoint(&self.checkpoint()?)?;
            self.stopped = true;
            return Ok(Some(Cut::BeforeResource));
        }
        match OPS[self.state.position] {
            Op::Allocate => {
                if self.state.list_call.is_none() {
                    let call = ListRequest {
                        identity: Arc::new(()),
                        checkpoint: self.checkpoint()?,
                    };
                    call.validate()?;
                    self.state.list_call = Some(Arc::new(call));
                }
                if !self.state.list_returned {
                    let call = self.state.list_call.as_ref().unwrap();
                    host.synchronize_checkpoint(call.checkpoint())?;
                    let reply = host.allocate_list(call)?;
                    // Move the actual reply into owned storage before ALL validation.
                    self.state.initial_list = match reply {
                        Allocated::Null => None,
                        Allocated::Bytes { token, data } => Some(Owner { token, data }),
                    };
                    self.state.list_returned = true;
                    self.state.list_receipt = self
                        .state
                        .initial_list
                        .as_ref()
                        .and_then(ListReceipt::capture);
                    self.state.validate()?;
                    return Ok(None);
                }
                let actual = self.state.initial_list.as_ref().unwrap();
                let mut image = self.list_buffer.take().expect("pre-reserved list image");
                image.token = actual.token;
                image.data.copy_from_slice(&actual.data);
                self.state.list = Some(image);
            }
            Op::Store(store) => {
                if let Value::Section(site) = store.value {
                    let at = site.index();
                    if self.state.section_calls[at].is_none() {
                        let call = SectionCall {
                            identity: Arc::new(()),
                            checkpoint: self.checkpoint()?,
                            site,
                        };
                        call.validate()?;
                        self.state.section_calls[at] = Some(Arc::new(call));
                    }
                    if self.state.sections[at].is_none() {
                        let call = self.state.section_calls[at].as_ref().unwrap();
                        host.synchronize_checkpoint(call.checkpoint())?;
                        self.state.sections[at] = Some(host.initialize_section(call)?);
                        self.state.receipts[at] = self.state.sections[at]
                            .as_ref()
                            .and_then(SectionReceipt::capture);
                        self.state.validate()?;
                        return Ok(None);
                    }
                }
                let (token, offset) = store.location(self.request(), self.state.list.as_ref())?;
                let (bytes, pc) = store.bytes(
                    &self.state.parent,
                    &self.state.caller,
                    self.state.list.as_ref(),
                    self.request(),
                    &self.state.sections,
                )?;
                let receipt = &mut self.state.history[write_count(self.state.position)];
                let owner = match store.place {
                    Place::Parent(_) => &mut self.state.parent,
                    Place::Caller(_) => &mut self.state.caller,
                    Place::List(_) => self.state.list.as_mut().unwrap(),
                };
                let current = &mut owner.data[offset..offset + store.width()];
                receipt.owner = token;
                receipt.offset = offset;
                receipt.instruction = pc;
                receipt.child_return = store.child_return();
                receipt.before.copy_from_slice(current);
                receipt.after.copy_from_slice(&bytes[..store.width()]);
                current.copy_from_slice(&bytes[..store.width()]);
            }
        }
        self.state.position += 1;
        Ok(None)
    }
}
