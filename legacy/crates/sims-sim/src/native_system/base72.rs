//! First provider role only: Base72 at +36 in its complete1240-byte parent,
//! called by5d59b7 with DWORD0. This cursor models the listed object/caller
//! effects and normal child return5d5a8a..5d5a99, not machine prologues, SEH,
//! exception unwinding, the derived tree suffix, or a completed provider.
use crate::{
    native_application::{InitializedSection, SectionRequest, Write},
    native_world_children::Owner,
    saved_matrix::{MAX_ALLOCATION_BYTES, Stop},
};
use std::sync::Arc;

#[cfg(test)]
mod tests;

const BASE: usize = 36;
const PARENT_BYTES: usize = 1240;
const IMPORT: usize = 11;
const RETURN: usize = OPS.len();
const DONE: usize = RETURN + 1;

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn end(owner: &Owner) -> Result<u64, Stop> {
    if owner.token == 0 || owner.data.is_empty() || owner.data.len() > MAX_ALLOCATION_BYTES {
        return Err(unsupported("system Base72 needs a complete bounded owner"));
    }
    owner.token.checked_add(owner.data.len() as u64)
        .filter(|end| *end <= 1_u64 << 32)
        .ok_or_else(|| unsupported("system Base72 owner exceeds native width"))
}

fn overlap(left: &Owner, right: &Owner) -> Result<bool, Stop> {
    Ok(left.token < end(right)? && right.token < end(left)?)
}

fn frame_offset(caller: &Owner, ebp: u64, relative: i32) -> Result<usize, Stop> {
    let address = ebp.checked_add_signed(i64::from(relative))
        .ok_or_else(|| unsupported("system Base72 caller address wraps"))?;
    usize::try_from(address.checked_sub(caller.token)
        .ok_or_else(|| unsupported("system Base72 caller slot precedes its owner"))?)
        .map_err(|_| unsupported("system Base72 caller offset exceeds the host profile"))
}

fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("admitted DWORD"))
}

fn admit(parent: &Owner, caller: &Owner, ebp: u64) -> Result<(), Stop> {
    end(parent)?;
    let caller_end = end(caller)?;
    let first = ebp.checked_sub(0x10)
        .ok_or_else(|| unsupported("system Base72 caller frame underflows"))?;
    let last = ebp.checked_add(12).filter(|end| *end <= 1_u64 << 32)
        .ok_or_else(|| unsupported("system Base72 caller frame exceeds native width"))?;
    if parent.data.len() != PARENT_BYTES || first < caller.token || last > caller_end {
        return Err(unsupported("system Base72 requires parent1240 and the full caller interval"));
    }
    if overlap(parent, caller)? {
        return Err(unsupported("system Base72 parent and complete caller owner alias"));
    }
    // Reserve room in the bounded ownership profile for a genuine debug return.
    if parent.data.len().checked_add(caller.data.len()).and_then(|n| n.checked_add(32))
        .is_none_or(|n| n > MAX_ALLOCATION_BYTES)
    {
        return Err(unsupported("system Base72 ownership exceeds its bounded profile"));
    }
    let argument = frame_offset(caller, ebp, 8)?;
    if word(&caller.data[argument..argument + 4]) != 0 {
        return Err(unsupported("this system Base72 role requires caller DWORD0"));
    }
    Ok(())
}

fn storage(bytes: usize) -> Result<Vec<u8>, Stop> {
    let mut result = Vec::new();
    result.try_reserve_exact(bytes)
        .map_err(|_| Stop::Unavailable("system Base72 snapshot/history storage unavailable".into()))?;
    result.resize(bytes, 0);
    Ok(result)
}

fn snapshot(owner: &Owner) -> Result<Owner, Stop> {
    let mut data = storage(owner.data.len())?;
    data.copy_from_slice(&owner.data);
    Ok(Owner { token: owner.token, data })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub parent: Owner,
    pub caller_frame: Owner,
    pub ebp: u64,
}

#[derive(Debug)]
pub struct BeginError {
    pub stop: Stop,
    pub incoming: Incoming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    Base72Returned,
}

pub trait Host {
    /// If an external effect precedes Err, retain it in the host and service an
    /// identical retry without issuing that effect again. Successful replies
    /// are staged in this cursor before any validation or image publication.
    fn initialize_section(&mut self, request: &Request) -> Result<InitializedSection, Stop>;
}

#[derive(Debug)]
struct Invocation;

/// Read-only snapshots sealed by the actual source cursor at import5d91bc.
/// Cloning retains this call's opaque identity; it cannot create another call.
#[derive(Debug, Clone)]
pub struct Request {
    invocation: Arc<Invocation>,
    initial_parent: Owner,
    parent: Owner,
    initial_caller_frame: Owner,
    caller_frame: Owner,
    ebp: u64,
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.same_call(other)
    }
}
impl Eq for Request {}

impl Request {
    pub fn initial_parent(&self) -> &Owner { &self.initial_parent }
    pub fn parent(&self) -> &Owner { &self.parent }
    pub fn initial_caller_frame(&self) -> &Owner { &self.initial_caller_frame }
    pub fn caller_frame(&self) -> &Owner { &self.caller_frame }
    pub fn ebp(&self) -> u64 { self.ebp }

    pub fn section_request(&self) -> SectionRequest {
        SectionRequest {
            instruction: 0x5d91bc,
            import_slot: 0x666114,
            region: Owner {
                token: self.parent.token + (BASE + 24) as u64,
                data: self.parent.data[BASE + 24..BASE + 48].to_vec(),
            },
        }
    }

    pub fn same_call(&self, other: &Request) -> bool {
        Arc::ptr_eq(&self.invocation, &other.invocation)
            && self.initial_parent == other.initial_parent && self.parent == other.parent
            && self.initial_caller_frame == other.initial_caller_frame
            && self.caller_frame == other.caller_frame && self.ebp == other.ebp
    }

    pub fn validate(&self) -> Result<(), Stop> {
        admit(&self.initial_parent, &self.initial_caller_frame, self.ebp)?;
        let mut parent = snapshot(&self.initial_parent)?;
        let mut caller = snapshot(&self.initial_caller_frame)?;
        for op in &OPS[..IMPORT] {
            let bytes = op.bytes(&parent, &caller, self.ebp, None)?;
            let (_, offset) = op.location(&parent, &caller, self.ebp)?;
            let owner = op.owner_mut(&mut parent, &mut caller);
            owner.data[offset..offset + op.width()].copy_from_slice(&bytes[..op.width()]);
        }
        if self.parent != parent || self.caller_frame != caller {
            return Err(unsupported("system Base72 request differs from the exact pre-import source images"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum Place { Parent(usize), Caller(i32) }
#[derive(Debug, Clone, Copy)]
enum Value { Word(u32), SelfPointer, ZeroSection, Section, Getter, Argument, ByteZero }
#[derive(Debug, Clone, Copy)]
struct Op { place: Place, value: Value, instruction: u32, child_return: bool }

impl Op {
    const fn word(member: usize, value: u32, instruction: u32) -> Self {
        Self { place: Place::Parent(member), value: Value::Word(value), instruction, child_return: false }
    }
    fn width(&self) -> usize {
        match self.value { Value::ZeroSection | Value::Section => 24, Value::ByteZero => 1, _ => 4 }
    }
    fn location(&self, parent: &Owner, caller: &Owner, ebp: u64) -> Result<(u64, usize), Stop> {
        match self.place {
            Place::Parent(member) => Ok((parent.token, BASE + member)),
            Place::Caller(relative) => Ok((caller.token, frame_offset(caller, ebp, relative)?)),
        }
    }
    fn owner_mut<'a>(&self, parent: &'a mut Owner, caller: &'a mut Owner) -> &'a mut Owner {
        match self.place { Place::Parent(_) => parent, Place::Caller(_) => caller }
    }
    fn bytes(&self, parent: &Owner, caller: &Owner, ebp: u64, section: Option<&InitializedSection>)
        -> Result<[u8; 24], Stop>
    {
        let mut bytes = [0; 24];
        let value = match self.value {
            Value::Word(value) => value,
            Value::SelfPointer => (parent.token + BASE as u64) as u32,
            Value::Getter => word(&parent.data[BASE + 48..BASE + 52]),
            Value::Argument => {
                let offset = frame_offset(caller, ebp, 8)?;
                word(&caller.data[offset..offset + 4])
            }
            Value::Section => {
                let returned = section.ok_or_else(|| unsupported("system Base72 lacks its actual import return"))?;
                if returned.region.data.len() != 24 {
                    return Err(unsupported("system Base72 import return has another extent"));
                }
                bytes.copy_from_slice(&returned.region.data);
                return Ok(bytes);
            }
            Value::ZeroSection | Value::ByteZero => return Ok(bytes),
        };
        bytes[..4].copy_from_slice(&value.to_le_bytes());
        Ok(bytes)
    }
}

// The first role's complete program, with no derived92 tree suffix. Every
// location is relative to the complete owner, not a separately owned Base72.
const OPS: [Op; 22] = [
    Op { place: Place::Caller(-0x10), value: Value::SelfPointer, instruction: 0x5d5a2f, child_return: false },
    Op::word(0, 0x673840, 0x5d5a32),
    Op::word(12, 0, 0x5d5a38),
    Op::word(8, 0x673720, 0x5d5a3c),
    Op::word(4, 0x673714, 0x5d5a43),
    Op::word(8, 0x6736fc, 0x5d5a4a),
    Op { place: Place::Caller(-4), value: Value::Word(0), instruction: 0x5d5a51, child_return: false },
    Op::word(48, 0, 0x5d91a7),
    Op::word(52, 0, 0x5d91aa),
    Op::word(20, 0x673ab8, 0x5d91ad),
    Op { place: Place::Parent(24), value: Value::ZeroSection, instruction: 0x5d91b3, child_return: true },
    Op { place: Place::Parent(24), value: Value::Section, instruction: 0x5d91bc, child_return: true },
    Op::word(48, 1, 0x5d91c2),
    Op::word(16, 0x673858, 0x5d9485),
    Op { place: Place::Parent(56), value: Value::Getter, instruction: 0x5d9492, child_return: false },
    Op { place: Place::Parent(64), value: Value::ByteZero, instruction: 0x5d5a62, child_return: false },
    Op::word(68, 0, 0x5d5a69),
    Op { place: Place::Parent(60), value: Value::Argument, instruction: 0x5d5a6d, child_return: false },
    Op::word(16, 0x673838, 0x5d5a70),
    Op::word(0, 0x673820, 0x5d5a76),
    Op::word(4, 0x673814, 0x5d5a7c),
    Op::word(8, 0x6737fc, 0x5d5a83),
];

fn validate_section(section: &InitializedSection, request: &Request) -> Result<(), Stop> {
    // Reuse the recovered DLL's bounded24/32 layout validator, including the
    // genuine nullable-debug branch, rather than accepting arbitrary images.
    section.validate(&request.section_request())?;
    if let Some(debug) = &section.debug_info {
        end(debug)?;
        if overlap(debug, request.parent())? || overlap(debug, request.caller_frame())? {
            return Err(unsupported("system section debug owner aliases the full parent/caller owner"));
        }
    }
    Ok(())
}

// A fixed-size receipt of the actual return, reserved inline before the host
// runs. It authenticates capability/debug identity as well as the image that
// appears in the write history. No post-effect snapshot allocation is needed.
#[derive(Debug, PartialEq, Eq)]
struct ReplyReceipt {
    token: u64,
    bytes: [u8; 24],
    capability: u64,
    debug: Option<(u64, [u8; 32])>,
}

impl ReplyReceipt {
    fn capture(reply: &InitializedSection) -> Option<Self> {
        let debug = match &reply.debug_info {
            Some(owner) => Some((owner.token, owner.data.as_slice().try_into().ok()?)),
            None => None,
        };
        Some(Self {
            token: reply.region.token,
            bytes: reply.region.data.as_slice().try_into().ok()?,
            capability: reply.capability,
            debug,
        })
    }
}

/// Owning, non-Clone and non-Deserialize cursor. Original input, current full
/// images, the sealed request, and any staged reply survive every typed stop.
#[derive(Debug)]
pub struct Construction {
    incoming: Incoming,
    invocation: Arc<Invocation>,
    parent: Owner,
    caller_frame: Owner,
    request: Request,
    sealed: bool,
    section: Option<InitializedSection>,
    reply_receipt: Option<ReplyReceipt>,
    // All22 before/after buffers and all full snapshots exist BEFORE the host
    // can run. Only history[..min(next,22)] has been applied and is exposed.
    history: Vec<Write>,
    next: usize,
}

impl Construction {
    pub fn new(incoming: Incoming) -> Result<Self, BeginError> {
        let prepare = || -> Result<_, Stop> {
            admit(&incoming.parent, &incoming.caller_frame, incoming.ebp)?;
            let invocation = Arc::new(Invocation);
            let request = Request {
                invocation: Arc::clone(&invocation),
                initial_parent: snapshot(&incoming.parent)?, parent: snapshot(&incoming.parent)?,
                initial_caller_frame: snapshot(&incoming.caller_frame)?,
                caller_frame: snapshot(&incoming.caller_frame)?, ebp: incoming.ebp,
            };
            let mut history = Vec::new();
            history.try_reserve_exact(OPS.len())
                .map_err(|_| Stop::Unavailable("system Base72 history unavailable".into()))?;
            for op in &OPS {
                let (owner, offset) = op.location(&incoming.parent, &incoming.caller_frame, incoming.ebp)?;
                history.push(Write { owner, offset, instruction: op.instruction,
                    child_return: op.child_return, before: storage(op.width())?, after: storage(op.width())? });
            }
            Ok((invocation, request, history, snapshot(&incoming.parent)?, snapshot(&incoming.caller_frame)?))
        };
        let (invocation, request, history, parent, caller_frame) = match prepare() {
            Ok(prepared) => prepared,
            Err(stop) => return Err(BeginError { stop, incoming }),
        };
        Ok(Self { incoming, invocation, parent, caller_frame, request, sealed: false,
            section: None, reply_receipt: None, history, next: 0 })
    }

    pub fn parent(&self) -> &Owner { &self.parent }
    pub fn caller_frame(&self) -> &Owner { &self.caller_frame }
    pub fn writes(&self) -> &[Write] { &self.history[..self.next.min(RETURN)] }
    pub fn request(&self) -> Option<&Request> { self.sealed.then_some(&self.request) }
    /// Includes a successful pending return, even if its validation failed.
    pub fn section(&self) -> Option<&InitializedSection> { self.section.as_ref() }
    pub fn returned(&self) -> bool { self.next == DONE }

    pub fn validate(&self) -> Result<(), Stop> {
        admit(&self.incoming.parent, &self.incoming.caller_frame, self.incoming.ebp)?;
        if self.next > DONE || self.history.len() != RETURN || self.sealed != (self.next >= IMPORT)
            || !Arc::ptr_eq(&self.invocation, &self.request.invocation)
            || self.request.initial_parent != self.incoming.parent
            || self.request.initial_caller_frame != self.incoming.caller_frame
            || self.request.ebp != self.incoming.ebp
        {
            return Err(unsupported("system Base72 source progress or call identity differs"));
        }
        if self.sealed {
            self.request.validate()?;
        } else if self.request.parent != self.incoming.parent
            || self.request.caller_frame != self.incoming.caller_frame
        {
            return Err(unsupported("system Base72 unsealed snapshot differs"));
        }
        if (self.next < IMPORT && self.section.is_some()) || (self.next > IMPORT && self.section.is_none()) {
            return Err(unsupported("system Base72 import return is outside its source position"));
        }
        if let Some(section) = &self.section {
            // Malformed extents are still retained in `section`; their absent
            // receipt cannot grant publication or a replacement host effect.
            let receipt = ReplyReceipt::capture(section)
                .ok_or_else(|| unsupported("system Base72 staged return has malformed image extents"))?;
            if self.reply_receipt.as_ref() != Some(&receipt) {
                return Err(unsupported("system Base72 return differs from its actual reply receipt"));
            }
            validate_section(section, &self.request)?;
        } else if self.reply_receipt.is_some() {
            return Err(unsupported("system Base72 reply receipt lost its owning return"));
        }
        let mut parent = snapshot(&self.incoming.parent)?;
        let mut caller = snapshot(&self.incoming.caller_frame)?;
        for (index, (op, receipt)) in OPS.iter().zip(&self.history).enumerate() {
            let (token, offset) = op.location(&parent, &caller, self.incoming.ebp)?;
            if receipt.owner != token || receipt.offset != offset || receipt.instruction != op.instruction
                || receipt.child_return != op.child_return
                || receipt.before.len() != op.width() || receipt.after.len() != op.width()
            {
                return Err(unsupported("system Base72 write receipt has another source or extent"));
            }
            if index < self.next.min(RETURN) {
                let bytes = op.bytes(&parent, &caller, self.incoming.ebp, self.section.as_ref())?;
                let owner = op.owner_mut(&mut parent, &mut caller);
                let current = &mut owner.data[offset..offset + op.width()];
                if &*current != receipt.before.as_slice() || receipt.after.as_slice() != &bytes[..op.width()] {
                    return Err(unsupported("system Base72 history differs from the exact source before/after"));
                }
                current.copy_from_slice(&bytes[..op.width()]);
            } else if receipt.before.iter().chain(&receipt.after).any(|byte| *byte != 0) {
                return Err(unsupported("system Base72 future history is already populated"));
            }
        }
        if self.parent != parent || self.caller_frame != caller {
            return Err(unsupported("system Base72 full owner images differ from source replay"));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.returned() { return Ok(Some(Cut::Base72Returned)); }
        if self.next == RETURN {
            self.next = DONE;
            return Ok(Some(Cut::Base72Returned));
        }
        if self.next == IMPORT && self.section.is_none() {
            // Move the successful reply into owned storage BEFORE the first
            // fallible validation. Invalid replies remain here on every retry.
            self.section = Some(host.initialize_section(&self.request)?);
            self.reply_receipt = ReplyReceipt::capture(self.section.as_ref().expect("just staged the reply"));
            self.validate()?;
            return Ok(None);
        }
        let op = OPS[self.next];
        let bytes = op.bytes(&self.parent, &self.caller_frame, self.incoming.ebp, self.section.as_ref())?;
        let (_, offset) = op.location(&self.parent, &self.caller_frame, self.incoming.ebp)?;
        let owner = op.owner_mut(&mut self.parent, &mut self.caller_frame);
        let current = &mut owner.data[offset..offset + op.width()];
        let receipt = &mut self.history[self.next];
        receipt.before.copy_from_slice(current);
        receipt.after.copy_from_slice(&bytes[..op.width()]);
        current.copy_from_slice(&bytes[..op.width()]);
        self.next += 1;
        if self.next == IMPORT {
            self.request.parent.data.copy_from_slice(&self.parent.data);
            self.request.caller_frame.data.copy_from_slice(&self.caller_frame.data);
            self.sealed = true;
        }
        Ok(None)
    }

    pub fn into_prefix(self) -> Result<Prefix, Self> {
        if !self.returned() || self.validate().is_err() { return Err(self); }
        Ok(Prefix { construction: self })
    }
}

/// Only an actual normal child completion can construct this immutable prefix.
/// It supplies no completed provider/global/Application/Game/World admission.
#[derive(Debug)]
pub struct Prefix { construction: Construction }

impl Prefix {
    pub fn parent(&self) -> &Owner { self.construction.parent() }
    pub fn caller_frame(&self) -> &Owner { self.construction.caller_frame() }
    pub fn request(&self) -> &Request { self.construction.request().expect("normal return sealed its call") }
    pub fn section(&self) -> &InitializedSection { self.construction.section().expect("normal return retained its import") }
    pub fn writes(&self) -> &[Write] { self.construction.writes() }
    pub fn validate(&self) -> Result<(), Stop> {
        self.construction.validate()?;
        if !self.construction.returned() {
            return Err(unsupported("system Base72 prefix lacks normal child return"));
        }
        Ok(())
    }
}
