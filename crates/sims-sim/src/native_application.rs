//! Recovered Base104 and Application324 prefix through the pending mutex/FPU
//! inputs, with persistent caller-frame operands from the post-SEH entry.
//! This is an owning source cursor, not a completed application producer.
//! Imported section bytes and their debug allocation form an explicit service
//! return. CoInitialize retains its actual host return and thread-bound lease;
//! Explicit version replies can advance to the pending named-mutex call. No constructor-return or Game handoff is provided,
//! and no Clone/Deserialize can manufacture a completed cursor.
use crate::{
    native_world::resources::strings,
    native_world_children::Owner,
    saved_matrix::{Allocated, MAX_ALLOCATION_BYTES, Stop},
};
use sims_vm::commander::Registry;
use std::collections::BTreeMap;

pub mod com;
pub mod fpu;
mod frame;
pub mod mutex;
pub mod platform;
mod program;
pub mod section;
use frame::Frame;
use program::{Op, Region};
#[cfg(test)]
mod tests;

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn span(owner: &Owner) -> Result<(u64, usize), Stop> {
    if owner.token == 0
        || owner.data.is_empty()
        || owner.data.len() > MAX_ALLOCATION_BYTES
        || owner
            .token
            .checked_add(owner.data.len() as u64)
            .is_none_or(|end| end > 1_u64 << 32)
    {
        return Err(unsupported(
            "application owner lacks a complete native-width extent",
        ));
    }
    Ok((owner.token, owner.data.len()))
}

fn disjoint(mut spans: Vec<(u64, usize)>) -> Result<(), Stop> {
    spans.sort_unstable();
    if spans
        .windows(2)
        .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
    {
        return Err(unsupported(
            "application constructor allocations alias retained ownership",
        ));
    }
    Ok(())
}

fn word(data: &[u8]) -> u32 {
    u32::from_le_bytes(data[..4].try_into().unwrap())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub application: Owner,
    /// Complete containing owners for argument bytes, caller stack and every
    /// existing Commander. Embedded views must not be registered separately.
    pub retained: Vec<Owner>,
    pub registry: Registry<u32>,
    pub registry_globals: Owner,
    /// Complete initial singleton storage; its preimage is preserved until the
    /// actual post-CoInitialize store, without inventing an initial zero.
    pub application_singleton: Owner,
    /// Both original CString-pointer words, not strings invented by this cursor.
    pub mutex_name_globals: Owner,
    pub mutex_singleton: Owner,
    /// Embedded12-byte descriptor at [ebp-2c]. Its complete retained caller
    /// owner must also contain the persistent operands through [ebp+c).
    pub mutex_temporary: u64,
    pub argument: u64,
    /// First and second pointer arguments to native55e371, respectively.
    pub tree_stack: [u64; 2],
}

impl Incoming {
    fn frame(&self) -> Result<Frame, Stop> {
        let frame = Frame::new(self.mutex_temporary)?;
        frame.containing(&self.retained)?;
        Ok(frame)
    }
    fn bytes(&self, token: u64) -> Result<&[u8], Stop> {
        self.retained
            .iter()
            .find_map(|owner| {
                let offset = usize::try_from(token.checked_sub(owner.token)?).ok()?;
                owner.data.get(offset..).filter(|bytes| !bytes.is_empty())
            })
            .ok_or_else(|| unsupported("application caller input has no complete containing owner"))
    }
    fn argument_bytes(&self) -> Result<&[u8], Stop> {
        self.c_string(self.argument)
    }
    fn c_string(&self, token: u64) -> Result<&[u8], Stop> {
        let bytes = self.bytes(token)?;
        let end = bytes
            .iter()
            .take(4097)
            .position(|byte| *byte == 0)
            .ok_or_else(|| {
                unsupported("application argument exceeds the bounded owned CString profile")
            })?;
        Ok(&bytes[..end])
    }
    pub fn validate(&self) -> Result<(), Stop> {
        if self.application.data.len() != 324
            || self.registry_globals.token != 0x6de858
            || self.registry_globals.data.len() != 8
            || self.application_singleton.token != 0x6f2a58
            || self.application_singleton.data.len() != 4
            || self.mutex_name_globals.token != 0x6c9f70
            || self.mutex_name_globals.data.len() != 8
            || self.mutex_singleton.token != 0x6f2a5c
            || self.mutex_singleton.data.len() != 4
            || self.retained.len() > 4096
        {
            return Err(unsupported(
                "application input needs its complete324 owner and registry globals",
            ));
        }
        self.registry.validate().map_err(Stop::Unsupported)?;
        if word(&self.registry_globals.data) != self.registry.head.unwrap_or(0)
            || word(&self.registry_globals.data[4..]) != self.registry.generation
        {
            return Err(unsupported(
                "application registry globals differ from its incoming registry",
            ));
        }
        let mut spans = vec![
            span(&self.application)?,
            span(&self.registry_globals)?,
            span(&self.application_singleton)?,
            span(&self.mutex_name_globals)?,
            span(&self.mutex_singleton)?,
        ];
        let mut total = 324 + 8 + 4 + 8 + 4;
        for owner in &self.retained {
            spans.push(span(owner)?);
            total += owner.data.len();
            if total > MAX_ALLOCATION_BYTES {
                return Err(unsupported(
                    "application ownership exceeds its bounded profile",
                ));
            }
        }
        disjoint(spans)?;
        self.argument_bytes()?;
        for token in self.tree_stack {
            self.bytes(token)?;
        }
        let frame = self.frame()?;
        let argument_slot = frame.address(8, 4)?;
        if word(self.bytes(argument_slot)?) != self.argument as u32 {
            return Err(unsupported(
                "application frame argument slot differs from its incoming argument",
            ));
        }
        for offset in [0, 4] {
            let token = u64::from(word(&self.mutex_name_globals.data[offset..]));
            let bytes = self.c_string(token)?;
            if token < self.mutex_temporary + 12
                && self.mutex_temporary < token + bytes.len() as u64 + 1
            {
                return Err(unsupported(
                    "mutex input CString aliases its mutable temporary descriptor",
                ));
            }
        }
        let identity = (self.application.token + 104) as u32;
        if self.registry.nodes.contains_key(&identity) {
            return Err(unsupported(
                "application Commander identity is already retained",
            ));
        }
        let temporary_container = self
            .retained
            .iter()
            .find(|owner| {
                self.mutex_temporary >= owner.token
                    && self.mutex_temporary + 12 <= owner.token + owner.data.len() as u64
            })
            .unwrap();
        for (identity, node) in &self.registry.nodes {
            // The temporary is mutable stack storage. Retained Commander
            // allocations, including their unmodeled bytes, remain unchanged.
            if u64::from(*identity) >= temporary_container.token
                && u64::from(*identity)
                    < temporary_container.token + temporary_container.data.len() as u64
            {
                return Err(unsupported(
                    "mutex temporary reuses a retained Commander's containing owner",
                ));
            }
            let bytes = self.bytes(u64::from(*identity))?;
            let expected: Vec<_> = [
                if node.handler == 0 {
                    0x666628
                } else {
                    node.handler
                },
                node.next.unwrap_or(0),
                node.auxiliary,
                node.serial,
            ]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
            if bytes.get(..16) != Some(expected.as_slice()) {
                return Err(unsupported(
                    "incoming Commander lacks its original containing bytes",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct BeginError {
    pub stop: Stop,
    pub incoming: Incoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionRequest {
    pub instruction: u32,
    pub import_slot: u32,
    pub region: Owner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitializedSection {
    /// The complete actual returned in/out image, not guessed native bytes.
    pub region: Owner,
    /// A live host capability retained with this cursor and its services.
    pub capability: u64,
    /// The complete separate32-byte debug allocation produced by the source
    /// routine. None is its actual nullable-allocation return path.
    pub debug_info: Option<Owner>,
}

pub trait Host: strings::Host {
    /// If an Err follows an external effect, the host must retain its pending
    /// reply and acknowledge the identical request without repeating that effect.
    fn initialize_section(&mut self, request: &SectionRequest) -> Result<InitializedSection, Stop>;
    fn allocate_tree(&mut self, bytes: u32) -> Result<Allocated, Stop>;
    fn initialize_com(&mut self, request: &com::Request) -> Result<com::Return, Stop>;
    fn query_version(
        &mut self,
        request: &platform::VersionRequest,
    ) -> Result<platform::VersionReturn, Stop>;
    fn allocate_mutex_wrapper(
        &mut self,
        request: &platform::WrapperRequest,
    ) -> Result<Allocated, Stop>;
    fn create_mutex(&mut self, _: &platform::MutexRequest) -> Result<mutex::Return, Stop> {
        Err(Stop::Unavailable(
            "no named-object authority was supplied".into(),
        ))
    }
    fn read_control(&mut self, _: &fpu::ReadRequest) -> Result<fpu::ReadReturn, Stop> {
        Err(Stop::Unavailable(
            "no application FPU control context was supplied".into(),
        ))
    }
    fn load_control(&mut self, _: &fpu::LoadRequest) -> Result<fpu::LoadReturn, Stop> {
        Err(Stop::Unavailable(
            "no application FPU load context was supplied".into(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    InitializeSection(SectionRequest),
    CoInitialize(com::Request),
    /// This query has no admitted backend return yet.
    GetVersion(platform::VersionRequest),
    AllocateMutexWrapper(platform::WrapperRequest),
    CreateMutex(platform::MutexRequest),
    ReadControlWord {
        instruction: u32,
    },
    LoadControlWord(fpu::LoadRequest),
    /// The source still needs its genuine global/vtable producer. This marks
    /// the unresolved provider boundary, not an already executed virtual call.
    ProviderVirtual {
        instruction: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    Base104Returned,
    CommanderReturned,
    DefaultStringsReturned,
    DerivedDispatchBound,
    ImportEntry,
    ComReturned,
    Published,
    VersionReturned,
    MutexEntry,
    NullMutexPublished,
    MutexReturned,
    FpuReturned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    pub owner: u64,
    pub offset: usize,
    pub instruction: u32,
    pub child_return: bool,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Effect {
    Section(SectionRequest, InitializedSection),
    Tree(u32, Allocated),
    String(strings::Request, Allocated),
    Com(com::Request, com::Return),
    Version(platform::VersionRequest, platform::VersionReturn),
    MutexAllocation(platform::WrapperRequest, Allocated),
    Mutex(platform::MutexRequest, mutex::Return),
    FreeString(u64),
    FpuRead(fpu::ReadRequest, fpu::ReadReturn),
    FpuLoad(fpu::LoadRequest, fpu::LoadReturn),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Progress {
    owner: Owner,
    globals: Owner,
    singleton: Owner,
    mutex_singleton: Owner,
    retained: Vec<Owner>,
    registry: Registry<u32>,
    tree: Option<Owner>,
    strings: BTreeMap<usize, strings::State>,
    section: Option<InitializedSection>,
    pending_section: Option<InitializedSection>,
    pending_tree: Option<Allocated>,
    com: Option<com::Return>,
    pending_com: Option<com::Return>,
    pending_version: Option<platform::VersionReturn>,
    version: Option<platform::VersionReturn>,
    pending_wrapper: Option<Allocated>,
    wrapper: Option<Owner>,
    pending_mutex: Option<mutex::Return>,
    mutex: Option<mutex::Return>,
    pending_fpu_read: Option<fpu::ReadReturn>,
    fpu_read: Option<fpu::ReadReturn>,
    fpu_original_dword: Option<u32>,
    pending_fpu_load: Option<fpu::LoadReturn>,
    fpu_load: Option<fpu::LoadReturn>,
    temporary: Option<strings::State>,
    child: Option<strings::Construction>,
    next: usize,
    cut: Option<Cut>,
    writes: Vec<Write>,
}

impl Progress {
    fn new(input: &Incoming) -> Self {
        Self {
            owner: input.application.clone(),
            globals: input.registry_globals.clone(),
            singleton: input.application_singleton.clone(),
            mutex_singleton: input.mutex_singleton.clone(),
            retained: input.retained.clone(),
            registry: input.registry.clone(),
            tree: None,
            strings: BTreeMap::new(),
            section: None,
            pending_section: None,
            pending_tree: None,
            com: None,
            pending_com: None,
            pending_version: None,
            version: None,
            pending_wrapper: None,
            wrapper: None,
            pending_mutex: None,
            mutex: None,
            pending_fpu_read: None,
            fpu_read: None,
            fpu_original_dword: None,
            pending_fpu_load: None,
            fpu_load: None,
            temporary: None,
            child: None,
            next: 0,
            cut: None,
            writes: vec![],
        }
    }
    fn bytes(&self, token: u64) -> Result<&[u8], Stop> {
        self.retained
            .iter()
            .find_map(|owner| {
                let offset = usize::try_from(token.checked_sub(owner.token)?).ok()?;
                owner.data.get(offset..).filter(|bytes| !bytes.is_empty())
            })
            .ok_or_else(|| unsupported("application operand has no retained caller owner"))
    }
    fn c_string(&self, token: u64) -> Result<&[u8], Stop> {
        let bytes = self.bytes(token)?;
        let end = bytes
            .iter()
            .take(4097)
            .position(|byte| *byte == 0)
            .ok_or_else(|| unsupported("application live CString exceeds its owned profile"))?;
        Ok(&bytes[..end])
    }
    fn write_caller(&mut self, input: &Incoming, relative: i32, bytes: &[u8], instruction: u32) {
        // Admission already bound every persistent slot to this one full owner.
        let frame = input.frame().unwrap();
        let address = frame.address(relative, bytes.len()).unwrap();
        let owner_token = frame.containing(&self.retained).unwrap().token;
        let owner = self
            .retained
            .iter_mut()
            .find(|owner| owner.token == owner_token)
            .unwrap();
        let offset = (address - owner.token) as usize;
        let before = owner.data[offset..offset + bytes.len()].to_vec();
        owner.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        self.writes.push(Write {
            owner: owner.token,
            offset,
            instruction,
            child_return: false,
            before,
            after: bytes.to_vec(),
        });
    }
    fn caller_bytes(&self, input: &Incoming, relative: i32, bytes: usize) -> Result<&[u8], Stop> {
        let address = input.frame()?.address(relative, bytes)?;
        self.bytes(address)?
            .get(..bytes)
            .ok_or_else(|| unsupported("application caller read exceeds its retained owner"))
    }
    fn write(
        &mut self,
        region: Region,
        offset: usize,
        bytes: &[u8],
        instruction: u32,
        child_return: bool,
    ) {
        let owner = match region {
            Region::Application => &mut self.owner,
            Region::Globals => &mut self.globals,
            Region::Singleton => &mut self.singleton,
            Region::MutexSingleton => &mut self.mutex_singleton,
            Region::MutexWrapper => self.wrapper.as_mut().unwrap(),
            Region::Tree => self.tree.as_mut().unwrap(),
        };
        let before = owner.data[offset..offset + bytes.len()].to_vec();
        owner.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        self.writes.push(Write {
            owner: owner.token,
            offset,
            instruction,
            child_return,
            before,
            after: bytes.to_vec(),
        });
    }
    fn section_request(&self) -> SectionRequest {
        SectionRequest {
            instruction: 0x5d91bc,
            import_slot: 0x666114,
            region: Owner {
                token: self.owner.token + 24,
                data: self.owner.data[24..48].to_vec(),
            },
        }
    }
    fn wrapper_request(&self) -> platform::WrapperRequest {
        platform::WrapperRequest::new(self.version.as_ref().unwrap())
    }
    fn stack_descriptor(&self, input: &Incoming) -> Owner {
        let containing = self
            .retained
            .iter()
            .find(|owner| {
                input.mutex_temporary >= owner.token
                    && input.mutex_temporary + 12 <= owner.token + owner.data.len() as u64
            })
            .unwrap();
        let offset = (input.mutex_temporary - containing.token) as usize;
        Owner {
            token: input.mutex_temporary,
            data: containing.data[offset..offset + 12].to_vec(),
        }
    }
    fn merge_temporary(&mut self, input: &Incoming, state: strings::State) {
        let containing = self
            .retained
            .iter_mut()
            .find(|owner| {
                input.mutex_temporary >= owner.token
                    && input.mutex_temporary + 12 <= owner.token + owner.data.len() as u64
            })
            .unwrap();
        let offset = (input.mutex_temporary - containing.token) as usize;
        let before = containing.data[offset..offset + 12].to_vec();
        containing.data[offset..offset + 12].copy_from_slice(&state.owner.data);
        self.writes.push(Write {
            owner: containing.token,
            offset,
            instruction: if self.version.as_ref().unwrap().nt_name() {
                0x4eed33
            } else {
                0x4eed81
            },
            child_return: true,
            before,
            after: state.owner.data.clone(),
        });
        self.temporary = Some(state);
    }
    fn mutex_request(&self) -> platform::MutexRequest {
        platform::MutexRequest::new(
            self.version.as_ref().unwrap(),
            &self.owner,
            self.wrapper.as_ref().unwrap(),
            self.temporary.as_ref().unwrap(),
            self.com.as_ref().unwrap(),
        )
    }
    fn fpu_read_request(&self, invocation: &fpu::Invocation) -> fpu::ReadRequest {
        fpu::ReadRequest::new(
            invocation,
            &self.owner,
            self.com.as_ref().unwrap(),
            self.mutex.as_ref(),
            self.wrapper.as_ref(),
        )
    }
    fn fpu_changed(&self) -> bool {
        fpu::changed(self.fpu_original_dword.unwrap() as u16)
    }
    fn fpu_load_request(&self, input: &Incoming) -> Result<fpu::LoadRequest, Stop> {
        let request = fpu::LoadRequest::new(self.fpu_read.as_ref().unwrap());
        let word = u16::from_le_bytes(self.caller_bytes(input, 10, 2)?.try_into().unwrap());
        if word != request.control() {
            return Err(unsupported(
                "FPU load operand differs from its current caller WORD",
            ));
        }
        Ok(request)
    }
    fn validate_spans(&self, input: &Incoming, ops: &[Op]) -> Result<(), Stop> {
        let mut spans = vec![
            span(&self.owner)?,
            span(&self.globals)?,
            span(&self.singleton)?,
            span(&self.mutex_singleton)?,
            span(&input.mutex_name_globals)?,
        ];
        for owner in &self.retained {
            spans.push(span(owner)?);
        }
        if let Some(wrapper) = &self.wrapper {
            spans.push(span(wrapper)?);
        }
        if let Some(Allocated::Bytes { token, data }) = &self.pending_wrapper {
            spans.push(span(&Owner {
                token: *token,
                data: data.clone(),
            })?);
        }
        if let Some(temporary) = &self.temporary {
            temporary.validate()?;
            spans.push(span(&temporary.backing)?);
        }
        if let Some(tree) = &self.tree {
            spans.push(span(tree)?);
        }
        if let Some(Allocated::Bytes { token, data }) = &self.pending_tree {
            spans.push(span(&Owner {
                token: *token,
                data: data.clone(),
            })?);
        }
        for string in self.strings.values() {
            string.validate()?;
            spans.push(span(&string.backing)?);
        }
        if let Some(child) = &self.child {
            if !matches!(ops.get(self.next), Some(Op::String { .. } | Op::MutexName)) {
                return Err(unsupported(
                    "application string child lost its call position",
                ));
            }
            spans.extend(child.allocation_spans()?);
        }
        for section in [&self.pending_section, &self.section].into_iter().flatten() {
            section.validate(&self.section_request())?;
            if let Some(debug_info) = &section.debug_info {
                spans.push(span(debug_info)?);
            }
        }
        let total = spans
            .iter()
            .try_fold(0_usize, |total, (_, bytes)| total.checked_add(*bytes))
            .ok_or_else(|| unsupported("application retained byte count overflows"))?;
        if total > MAX_ALLOCATION_BYTES {
            return Err(unsupported(
                "application retained bytes exceed its bounded profile",
            ));
        }
        disjoint(spans)
    }
    fn advance(
        &mut self,
        input: &Incoming,
        com_request: &com::Request,
        version_request: &platform::VersionRequest,
        fpu_invocation: &fpu::Invocation,
        op: &Op,
        host: &mut impl Host,
    ) -> Result<(), Stop> {
        if matches!(op, Op::Boundary) {
            return Err(Stop::Unavailable(
                "Application324 is waiting at provider global6fe90c/virtual+34; no provider or constructor return is admitted"
                    .into(),
            ));
        }
        if matches!(op, Op::CoInitialize) && self.pending_com.is_none() {
            // Preserve the last cut if the call has not returned. A successful
            // reply is staged before validation and never requested twice.
            let reply = host.initialize_com(com_request)?;
            self.cut = None;
            self.pending_com = Some(reply);
            return Ok(());
        }
        if matches!(op, Op::Version) && self.pending_version.is_none() {
            let reply = host.query_version(version_request)?;
            self.cut = None;
            self.pending_version = Some(reply);
            return Ok(());
        }
        if matches!(op, Op::MutexAllocate) && self.pending_wrapper.is_none() {
            let reply = host.allocate_mutex_wrapper(&self.wrapper_request())?;
            self.cut = None;
            self.pending_wrapper = Some(reply);
            return Ok(());
        }
        if matches!(op, Op::MutexCreate) && self.wrapper.is_some() && self.pending_mutex.is_none() {
            let reply = host.create_mutex(&self.mutex_request())?;
            self.cut = None;
            self.pending_mutex = Some(reply);
            return Ok(());
        }
        if matches!(op, Op::FpuRead) && self.pending_fpu_read.is_none() {
            let returned = host.read_control(&self.fpu_read_request(fpu_invocation))?;
            self.cut = None;
            self.pending_fpu_read = Some(returned);
            return Ok(());
        }
        if matches!(op, Op::FpuLoad) && self.fpu_changed() && self.pending_fpu_load.is_none() {
            let returned = host.load_control(&self.fpu_load_request(input)?)?;
            self.cut = None;
            self.pending_fpu_load = Some(returned);
            return Ok(());
        }
        self.cut = None;
        match op {
            Op::CallerApplication => self.write_caller(
                input,
                -0x14,
                &(self.owner.token as u32).to_le_bytes(),
                0x4eebb1,
            ),
            Op::CallerStore {
                relative,
                bytes,
                pc,
            } => self.write_caller(input, *relative, bytes, *pc),
            Op::Store {
                region,
                offset,
                bytes,
                pc,
            } => self.write(*region, *offset, bytes, *pc, false),
            Op::Memset { offset, bytes, pc } => {
                self.write(Region::Application, *offset, &vec![0; *bytes], *pc, true)
            }
            Op::Section => {
                if let Some(reply) = self.pending_section.take() {
                    self.write(Region::Application, 24, &reply.region.data, 0x5d91bc, true);
                    self.section = Some(reply);
                } else {
                    self.pending_section = Some(host.initialize_section(&self.section_request())?);
                    return Ok(());
                }
            }
            Op::Tree => {
                if let Some(reply) = self.pending_tree.as_ref() {
                    let Allocated::Bytes { token, data } = reply else {
                        return Err(Stop::Unavailable(
                            "application tree allocation is null".into(),
                        ));
                    };
                    if data.len() != 24 {
                        return Err(unsupported(
                            "application tree allocation has another extent",
                        ));
                    }
                    self.tree = Some(Owner {
                        token: *token,
                        data: data.clone(),
                    });
                    self.pending_tree = None;
                } else {
                    self.pending_tree = Some(host.allocate_tree(24)?);
                    return Ok(());
                }
            }
            Op::TreePointer { region, offset, pc } => self.write(
                *region,
                *offset,
                &(self.tree.as_ref().unwrap().token as u32).to_le_bytes(),
                *pc,
                false,
            ),
            Op::StackByte { offset, pc } => self.write(
                Region::Application,
                *offset,
                &[self.bytes(input.tree_stack[0])?[0]],
                *pc,
                false,
            ),
            Op::String {
                offset,
                copy_argument,
                pc,
            } => {
                if self.child.is_none() {
                    self.child = Some(strings::Construction::new(
                        Owner {
                            token: self.owner.token + *offset as u64,
                            data: self.owner.data[*offset..*offset + 12].to_vec(),
                        },
                        if *copy_argument {
                            Some(self.c_string(input.argument)?.to_vec())
                        } else {
                            None
                        },
                    )?);
                    return Ok(());
                }
                if !self.child.as_mut().unwrap().step(host)? {
                    return Ok(());
                }
                let (descriptor, backing) = self.child.as_ref().unwrap().returned_parts()?.unwrap();
                let state = strings::State {
                    owner: descriptor.clone(),
                    backing: backing.clone(),
                };
                self.write(Region::Application, *offset, &state.owner.data, *pc, true);
                self.strings.insert(*offset, state);
                self.child = None;
            }
            Op::Commander => {
                let identity = (self.owner.token + 104) as u32;
                self.registry
                    .construct(identity)
                    .map_err(Stop::Unsupported)?;
                let node = &self.registry.nodes[&identity];
                let next = node.next.unwrap_or(0);
                let serial = node.serial;
                let generation = self.registry.generation;
                for (offset, value, pc) in [
                    (112, 0, 0x407579),
                    (104, 0x666628, 0x40757d),
                    (108, next, 0x407589),
                    (116, serial, 0x407592),
                ] {
                    self.write(Region::Application, offset, &value.to_le_bytes(), pc, false);
                }
                self.write(
                    Region::Globals,
                    4,
                    &generation.to_le_bytes(),
                    0x407595,
                    false,
                );
                self.write(Region::Globals, 0, &identity.to_le_bytes(), 0x40759b, false);
            }
            Op::Bind => {
                self.registry
                    .bind((self.owner.token + 104) as u32, 0x669804)
                    .map_err(Stop::Unsupported)?;
                self.write(
                    Region::Application,
                    104,
                    &0x669804_u32.to_le_bytes(),
                    0x4eecfc,
                    false,
                );
            }
            Op::CoInitialize => self.com = self.pending_com.take(),
            Op::Publish => self.write(
                Region::Singleton,
                0,
                &(self.owner.token as u32).to_le_bytes(),
                0x4eed08,
                false,
            ),
            Op::Version => self.version = self.pending_version.take(),
            Op::MutexAllocate => {
                match self.pending_wrapper.as_ref().unwrap() {
                    Allocated::Null => {}
                    Allocated::Bytes { token, data } => {
                        if data.len() != 12 {
                            return Err(unsupported("mutex wrapper allocation has another extent"));
                        }
                        self.wrapper = Some(Owner {
                            token: *token,
                            data: data.clone(),
                        });
                    }
                }
                let nt = self.version.as_ref().unwrap().nt_name();
                let pointer = self.wrapper.as_ref().map_or(0, |owner| owner.token as u32);
                self.write_caller(
                    input,
                    8,
                    &pointer.to_le_bytes(),
                    if nt { 0x4eed1f } else { 0x4eed6d },
                );
                self.write_caller(
                    input,
                    -4,
                    &[if nt { 8 } else { 11 }],
                    if nt { 0x4eed24 } else { 0x4eed72 },
                );
                self.pending_wrapper = None;
            }
            Op::MutexName => {
                if self.wrapper.is_some() {
                    if self.child.is_none() {
                        let offset = if self.version.as_ref().unwrap().nt_name() {
                            0
                        } else {
                            4
                        };
                        let token = u64::from(word(&input.mutex_name_globals.data[offset..]));
                        let name = self.c_string(token)?.to_vec();
                        if token < input.mutex_temporary + 12
                            && input.mutex_temporary < token + name.len() as u64 + 1
                        {
                            return Err(unsupported(
                                "live mutex CString aliases its temporary descriptor",
                            ));
                        }
                        self.child = Some(strings::Construction::new(
                            self.stack_descriptor(input),
                            Some(name),
                        )?);
                        return Ok(());
                    }
                    if !self.child.as_mut().unwrap().step(host)? {
                        return Ok(());
                    }
                    let (owner, backing) = self.child.as_ref().unwrap().returned_parts()?.unwrap();
                    let state = strings::State {
                        owner: owner.clone(),
                        backing: backing.clone(),
                    };
                    self.merge_temporary(input, state);
                    self.child = None;
                }
            }
            Op::MutexPreparation => {
                if self.wrapper.is_some() {
                    let nt = self.version.as_ref().unwrap().nt_name();
                    self.write_caller(
                        input,
                        -0x10,
                        &(if nt { 1_u32 } else { 2 }).to_le_bytes(),
                        if nt { 0x4eed38 } else { 0x4eed86 },
                    );
                    self.write_caller(
                        input,
                        -4,
                        &[if nt { 9 } else { 12 }],
                        if nt { 0x4eed43 } else { 0x4eed91 },
                    );
                }
            }
            Op::MutexZero => {
                if self.wrapper.is_some() {
                    self.write(
                        Region::MutexWrapper,
                        8,
                        &0_u32.to_le_bytes(),
                        0x5d94f9,
                        false,
                    );
                } else {
                    let pc = if self.version.as_ref().unwrap().nt_name() {
                        0x4eed54
                    } else {
                        0x4eeda2
                    };
                    self.write(Region::MutexSingleton, 0, &0_u32.to_le_bytes(), pc, false);
                    self.write_caller(
                        input,
                        -4,
                        &7_u32.to_le_bytes(),
                        if self.version.as_ref().unwrap().nt_name() {
                            0x4eed59
                        } else {
                            0x4eeda7
                        },
                    );
                }
            }
            Op::MutexVptr => {
                if self.wrapper.is_some() {
                    self.write(
                        Region::MutexWrapper,
                        0,
                        &0x673b18_u32.to_le_bytes(),
                        0x5d94fd,
                        false,
                    );
                }
            }
            Op::MutexCut => {
                self.cut = Some(if self.wrapper.is_some() {
                    Cut::MutexEntry
                } else {
                    Cut::NullMutexPublished
                })
            }
            Op::MutexCreate => {
                if self.wrapper.is_some() {
                    let reply = self.pending_mutex.as_ref().unwrap();
                    reply.validate(&self.mutex_request())?;
                    let returned = reply.clone();
                    let word = returned.word();
                    self.write(
                        Region::MutexWrapper,
                        4,
                        &word.to_le_bytes(),
                        0x5d9511,
                        false,
                    );
                    if word != 0 {
                        self.write(
                            Region::MutexWrapper,
                            8,
                            &1_u32.to_le_bytes(),
                            0x5d9516,
                            false,
                        );
                    }
                    self.mutex = Some(returned);
                    self.pending_mutex = None;
                }
            }
            Op::MutexPublish => {
                if let Some(wrapper) = &self.wrapper {
                    let pointer = (wrapper.token as u32).to_le_bytes();
                    let nt = self.version.as_ref().unwrap().nt_name();
                    self.write(
                        Region::MutexSingleton,
                        0,
                        &pointer,
                        if nt { 0x4eed54 } else { 0x4eeda2 },
                        false,
                    );
                    self.write_caller(
                        input,
                        -4,
                        &7_u32.to_le_bytes(),
                        if nt { 0x4eed59 } else { 0x4eeda7 },
                    );
                }
            }
            Op::MutexTemporaryDestroy => {
                if let Some(temporary) = &self.temporary {
                    // 5b8873/4016f8 releases backing only. The caller's copied
                    // descriptor deliberately remains dangling and unchanged.
                    host.free_string(temporary.backing.token)?;
                    self.temporary = None;
                }
            }
            Op::MutexReturnedCut => {
                self.write(Region::Application, 248, &[0], 0x4eedb8, false);
                self.cut = Some(if self.wrapper.is_some() {
                    Cut::MutexReturned
                } else {
                    Cut::NullMutexPublished
                });
            }
            Op::FpuRead => {
                let returned = self.pending_fpu_read.as_ref().unwrap();
                returned.validate(&self.fpu_read_request(fpu_invocation))?;
                let returned = returned.clone();
                self.write_caller(input, -0x10, &returned.raw().to_le_bytes(), 0x4eedc7);
                self.fpu_read = Some(returned);
                self.pending_fpu_read = None;
            }
            Op::FpuInspect => {
                // 4eedca reads the whole current DWORD after FNSTCW stored
                // only its low WORD. Keep the preserved high bytes in history.
                self.fpu_original_dword = Some(u32::from_le_bytes(
                    self.caller_bytes(input, -0x10, 4)?.try_into().unwrap(),
                ));
            }
            Op::FpuNormalize => {
                if self.fpu_changed() {
                    let normalized = fpu::normalized(self.fpu_original_dword.unwrap() as u16);
                    self.write_caller(input, 10, &normalized.to_le_bytes(), 0x4eedf1);
                }
            }
            Op::FpuLoad => {
                if self.fpu_changed() {
                    let returned = self.pending_fpu_load.as_ref().unwrap();
                    returned.validate(&self.fpu_load_request(input)?)?;
                    self.fpu_load = Some(returned.clone());
                    self.pending_fpu_load = None;
                }
            }
            Op::FpuOriginal => self.write(
                Region::Application,
                254,
                &(self.fpu_original_dword.unwrap() as u16).to_le_bytes(),
                0x4eedff,
                false,
            ),
            Op::FpuChanged => self.write(
                Region::Application,
                256,
                &u32::from(self.fpu_changed()).to_le_bytes(),
                0x4eee02,
                false,
            ),
            Op::Cut(cut) => self.cut = Some(*cut),
            Op::Boundary => unreachable!(),
        }
        self.next += 1;
        Ok(())
    }
}

struct Recording<'a, H> {
    host: &'a mut H,
    effects: &'a mut Vec<Effect>,
}
impl<H: Host> strings::Host for Recording<'_, H> {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        let reply = self.host.allocate_string(request)?;
        self.effects.push(Effect::String(request, reply.clone()));
        Ok(reply)
    }
    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        self.host.free_string(token)?;
        self.effects.push(Effect::FreeString(token));
        Ok(())
    }
}
impl<H: Host> Host for Recording<'_, H> {
    fn read_control(&mut self, request: &fpu::ReadRequest) -> Result<fpu::ReadReturn, Stop> {
        let returned = self.host.read_control(request)?;
        self.effects
            .push(Effect::FpuRead(request.clone(), returned.clone()));
        Ok(returned)
    }
    fn load_control(&mut self, request: &fpu::LoadRequest) -> Result<fpu::LoadReturn, Stop> {
        let returned = self.host.load_control(request)?;
        self.effects
            .push(Effect::FpuLoad(request.clone(), returned.clone()));
        Ok(returned)
    }
    fn create_mutex(&mut self, request: &platform::MutexRequest) -> Result<mutex::Return, Stop> {
        let reply = self.host.create_mutex(request)?;
        self.effects
            .push(Effect::Mutex(request.clone(), reply.clone()));
        Ok(reply)
    }
    fn query_version(
        &mut self,
        request: &platform::VersionRequest,
    ) -> Result<platform::VersionReturn, Stop> {
        let reply = self.host.query_version(request)?;
        self.effects
            .push(Effect::Version(request.clone(), reply.clone()));
        Ok(reply)
    }
    fn allocate_mutex_wrapper(
        &mut self,
        request: &platform::WrapperRequest,
    ) -> Result<Allocated, Stop> {
        let reply = self.host.allocate_mutex_wrapper(request)?;
        self.effects
            .push(Effect::MutexAllocation(request.clone(), reply.clone()));
        Ok(reply)
    }
    fn initialize_section(&mut self, request: &SectionRequest) -> Result<InitializedSection, Stop> {
        let reply = self.host.initialize_section(request)?;
        self.effects
            .push(Effect::Section(request.clone(), reply.clone()));
        Ok(reply)
    }
    fn allocate_tree(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        let reply = self.host.allocate_tree(bytes)?;
        self.effects.push(Effect::Tree(bytes, reply.clone()));
        Ok(reply)
    }
    fn initialize_com(&mut self, request: &com::Request) -> Result<com::Return, Stop> {
        let reply = self.host.initialize_com(request)?;
        self.effects
            .push(Effect::Com(request.clone(), reply.clone()));
        Ok(reply)
    }
}

struct Replay<'a> {
    effects: std::slice::Iter<'a, Effect>,
}
impl strings::Host for Replay<'_> {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        match self.effects.next() {
            Some(Effect::String(expected, reply)) if *expected == request => Ok(reply.clone()),
            _ => Err(unsupported("application string reply history differs")),
        }
    }
    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        match self.effects.next() {
            Some(Effect::FreeString(expected)) if *expected == token => Ok(()),
            _ => Err(unsupported("application string release history differs")),
        }
    }
}
impl Host for Replay<'_> {
    fn read_control(&mut self, request: &fpu::ReadRequest) -> Result<fpu::ReadReturn, Stop> {
        match self.effects.next() {
            Some(Effect::FpuRead(expected, returned)) if expected == request => {
                Ok(returned.clone())
            }
            _ => Err(unsupported("application FPU read history differs")),
        }
    }
    fn load_control(&mut self, request: &fpu::LoadRequest) -> Result<fpu::LoadReturn, Stop> {
        match self.effects.next() {
            Some(Effect::FpuLoad(expected, returned)) if expected == request => {
                Ok(returned.clone())
            }
            _ => Err(unsupported("application FPU load history differs")),
        }
    }
    fn create_mutex(&mut self, request: &platform::MutexRequest) -> Result<mutex::Return, Stop> {
        match self.effects.next() {
            Some(Effect::Mutex(expected, reply)) if expected == request => Ok(reply.clone()),
            _ => Err(unsupported("application mutex reply history differs")),
        }
    }
    fn query_version(
        &mut self,
        request: &platform::VersionRequest,
    ) -> Result<platform::VersionReturn, Stop> {
        match self.effects.next() {
            Some(Effect::Version(expected, reply)) if expected == request => Ok(reply.clone()),
            _ => Err(unsupported("application version reply history differs")),
        }
    }
    fn allocate_mutex_wrapper(
        &mut self,
        request: &platform::WrapperRequest,
    ) -> Result<Allocated, Stop> {
        match self.effects.next() {
            Some(Effect::MutexAllocation(expected, reply)) if expected == request => {
                Ok(reply.clone())
            }
            _ => Err(unsupported("application mutex allocation history differs")),
        }
    }
    fn initialize_section(&mut self, request: &SectionRequest) -> Result<InitializedSection, Stop> {
        match self.effects.next() {
            Some(Effect::Section(expected, reply)) if expected == request => Ok(reply.clone()),
            _ => Err(unsupported("application section reply history differs")),
        }
    }
    fn allocate_tree(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        match self.effects.next() {
            Some(Effect::Tree(expected, reply)) if *expected == bytes => Ok(reply.clone()),
            _ => Err(unsupported("application tree reply history differs")),
        }
    }
    fn initialize_com(&mut self, request: &com::Request) -> Result<com::Return, Stop> {
        match self.effects.next() {
            Some(Effect::Com(expected, reply)) if expected == request => Ok(reply.clone()),
            _ => Err(unsupported("application COM reply history differs")),
        }
    }
}

#[derive(Debug)]
pub struct Construction {
    incoming: Incoming,
    progress: Progress,
    effects: Vec<Effect>,
    steps: u32,
    com_request: com::Request,
    version_request: platform::VersionRequest,
    fpu_invocation: fpu::Invocation,
}

impl Construction {
    pub fn new(incoming: Incoming) -> Result<Self, BeginError> {
        if let Err(stop) = incoming.validate() {
            return Err(BeginError { stop, incoming });
        }
        Ok(Self {
            com_request: com::Request::new(incoming.application.token),
            version_request: platform::VersionRequest::new(incoming.application.token),
            fpu_invocation: fpu::Invocation::new(incoming.application.token),
            progress: Progress::new(&incoming),
            incoming,
            effects: vec![],
            steps: 0,
        })
    }
    pub fn owner(&self) -> &Owner {
        &self.progress.owner
    }
    pub fn registry(&self) -> &Registry<u32> {
        &self.progress.registry
    }
    pub fn registry_globals(&self) -> &Owner {
        &self.progress.globals
    }
    pub fn application_singleton(&self) -> &Owner {
        &self.progress.singleton
    }
    pub fn com_initialization(&self) -> Option<&com::Return> {
        self.progress.com.as_ref()
    }
    pub fn version(&self) -> Option<&platform::VersionReturn> {
        self.progress.version.as_ref()
    }
    pub fn mutex_wrapper(&self) -> Option<&Owner> {
        self.progress.wrapper.as_ref()
    }
    pub fn mutex_return(&self) -> Option<&mutex::Return> {
        self.progress.mutex.as_ref()
    }
    pub fn fpu_read(&self) -> Option<&fpu::ReadReturn> {
        self.progress.fpu_read.as_ref()
    }
    pub fn fpu_load(&self) -> Option<&fpu::LoadReturn> {
        self.progress.fpu_load.as_ref()
    }
    pub fn pending_fpu_read_request(&self) -> Option<fpu::ReadRequest> {
        matches!(
            program::operations().get(self.progress.next),
            Some(Op::FpuRead)
        )
        .then(|| self.progress.fpu_read_request(&self.fpu_invocation))
    }
    pub fn mutex_singleton(&self) -> &Owner {
        &self.progress.mutex_singleton
    }
    pub fn retained_callers(&self) -> &[Owner] {
        &self.progress.retained
    }
    pub fn section(&self) -> Option<&InitializedSection> {
        self.progress.section.as_ref()
    }
    pub fn writes(&self) -> &[Write] {
        &self.progress.writes
    }
    pub fn cut(&self) -> Option<Cut> {
        self.progress.cut
    }
    pub fn pending_call(&self) -> Option<Call> {
        match program::operations().get(self.progress.next) {
            Some(Op::Section) => Some(Call::InitializeSection(self.progress.section_request())),
            Some(Op::CoInitialize) => Some(Call::CoInitialize(self.com_request.clone())),
            Some(Op::Version) => Some(Call::GetVersion(self.version_request.clone())),
            Some(Op::MutexAllocate) => {
                Some(Call::AllocateMutexWrapper(self.progress.wrapper_request()))
            }
            Some(Op::MutexCreate) if self.progress.wrapper.is_some() => {
                Some(Call::CreateMutex(self.progress.mutex_request()))
            }
            Some(Op::FpuRead) => Some(Call::ReadControlWord {
                instruction: 0x4eedc7,
            }),
            Some(Op::FpuLoad) if self.progress.fpu_changed() => self
                .progress
                .fpu_load_request(&self.incoming)
                .ok()
                .map(Call::LoadControlWord),
            Some(Op::Boundary) => Some(Call::ProviderVirtual {
                instruction: 0x4eee29,
            }),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.incoming.validate()?;
        self.com_request.validate()?;
        self.version_request.validate()?;
        if self.com_request.application != self.incoming.application.token {
            return Err(unsupported(
                "application COM invocation lost its containing owner",
            ));
        }
        if self.version_request.application != self.incoming.application.token {
            return Err(unsupported(
                "application version invocation lost its containing owner",
            ));
        }
        if self.steps > 4096 {
            return Err(unsupported(
                "application prefix exceeds its progress profile",
            ));
        }
        let ops = program::operations();
        let mut progress = Progress::new(&self.incoming);
        let mut replay = Replay {
            effects: self.effects.iter(),
        };
        for _ in 0..self.steps {
            progress.validate_spans(&self.incoming, &ops)?;
            let op = ops
                .get(progress.next)
                .ok_or_else(|| unsupported("application prefix progress is outside its program"))?;
            progress.advance(
                &self.incoming,
                &self.com_request,
                &self.version_request,
                &self.fpu_invocation,
                op,
                &mut replay,
            )?;
        }
        if replay.effects.next().is_some() || progress != self.progress {
            return Err(unsupported(
                "application prefix differs from its exact source/reply/progress history",
            ));
        }
        for reply in [&self.progress.pending_com, &self.progress.com]
            .into_iter()
            .flatten()
        {
            reply.validate(&self.com_request)?;
        }
        for reply in [&self.progress.pending_version, &self.progress.version]
            .into_iter()
            .flatten()
        {
            reply.validate(&self.version_request)?;
        }
        if let Some(Call::CreateMutex(request)) = self.pending_call() {
            request.validate()?;
            if let Some(returned) = &self.progress.pending_mutex {
                returned.validate(&request)?;
            }
        }
        if let Some(returned) = &self.progress.mutex {
            returned.validate(returned.request())?;
        }
        if let Some(returned) = &self.progress.pending_fpu_read {
            returned.validate(&self.progress.fpu_read_request(&self.fpu_invocation))?;
        }
        if let Some(returned) = &self.progress.fpu_read {
            returned.validate(returned.request())?;
        }
        if let Some(returned) = &self.progress.pending_fpu_load {
            returned.validate(&self.progress.fpu_load_request(&self.incoming)?)?;
        }
        if let Some(returned) = &self.progress.fpu_load {
            returned.validate(returned.request())?;
        }
        self.progress.validate_spans(&self.incoming, &ops)
    }
    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        let ops = program::operations();
        self.progress.advance(
            &self.incoming,
            &self.com_request,
            &self.version_request,
            &self.fpu_invocation,
            &ops[self.progress.next],
            &mut Recording {
                host,
                effects: &mut self.effects,
            },
        )?;
        self.steps += 1;
        self.validate()?;
        Ok(self.cut())
    }
    /// This is only the still-pending prefix. It cannot supply a completed
    /// Application or an admitted Game/World input.
    pub fn into_prefix(self) -> Result<Prefix, Self> {
        if self.validate().is_err()
            || !matches!(
                self.pending_call(),
                Some(
                    Call::CoInitialize(_)
                        | Call::GetVersion(_)
                        | Call::AllocateMutexWrapper(_)
                        | Call::CreateMutex(_)
                        | Call::ReadControlWord { .. }
                        | Call::LoadControlWord(_)
                        | Call::ProviderVirtual { .. }
                )
            )
        {
            return Err(self);
        }
        Ok(Prefix { construction: self })
    }
}

#[derive(Debug)]
pub struct Prefix {
    construction: Construction,
}
impl Prefix {
    pub fn construction(&self) -> &Construction {
        &self.construction
    }
    pub fn into_construction(self) -> Construction {
        self.construction
    }
}
