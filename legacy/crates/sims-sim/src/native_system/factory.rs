//! Factory604850 and the first two children of5d5822, through5d59b7.
//! Supplied caller images model only the explicit operands recorded here, not
//! machine prologues, nested exception frames or an original process epoch.
use super::base72;
use crate::{
    native_application::Write,
    native_world_children::Owner,
    saved_matrix::{Allocated, MAX_ALLOCATION_BYTES, Stop},
};
use std::sync::Arc;

/// Opaque lifetime minted by a logical heap before the first constructor call.
/// An unrelated value cannot match an existing heap. Only retained clones
/// compare equal; there is no serialization or image-adoption operation.
#[derive(Debug, Clone)]
pub struct HeapOrigin(Arc<()>);
impl HeapOrigin {
    pub fn new() -> Self {
        Self(Arc::new(()))
    }
}
impl PartialEq for HeapOrigin {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for HeapOrigin {}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}
fn span(owner: &Owner) -> Result<(u64, u64), Stop> {
    let end = owner.token.checked_add(owner.data.len() as u64);
    if owner.token == 0
        || owner.data.is_empty()
        || owner.data.len() > MAX_ALLOCATION_BYTES
        || end.is_none_or(|end| end > 1_u64 << 32)
    {
        return Err(unsupported(
            "system constructor needs complete bounded native-width owners",
        ));
    }
    Ok((owner.token, end.unwrap()))
}
fn disjoint(owners: &[&Owner]) -> Result<(), Stop> {
    let mut spans = owners
        .iter()
        .map(|owner| span(owner))
        .collect::<Result<Vec<_>, _>>()?;
    spans.sort_unstable();
    if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(unsupported("system constructor retained owners alias"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frames {
    pub factory: u64,
    pub derived: u64,
    pub base: u64,
    pub refcount: u64,
    pub child: u64,
    pub base72: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub origin: HeapOrigin,
    pub caller: Owner,
    pub frames: Frames,
}
impl Incoming {
    pub fn validate(&self) -> Result<(), Stop> {
        let (start, end) = span(&self.caller)?;
        let mut ranges = Vec::new();
        for (ebp, low, high) in [
            (self.frames.factory, 16, 0),
            (self.frames.derived, 16, 0),
            (self.frames.base, 16, 0),
            (self.frames.refcount, 2, 0),
            (self.frames.child, 20, 0),
            (self.frames.base72, 16, 12),
        ] {
            let first = ebp
                .checked_sub(low)
                .ok_or_else(|| unsupported("system frame starts outside native width"))?;
            let last = ebp
                .checked_add(high)
                .ok_or_else(|| unsupported("system frame ends outside native width"))?;
            if first < start || last > end {
                return Err(unsupported(
                    "system frame operands lack one complete caller owner",
                ));
            }
            ranges.push((first, last));
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(unsupported(
                "simultaneously live system frame operands alias",
            ));
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct BeginError {
    pub stop: Stop,
    pub incoming: Incoming,
}

/// Only Construction can create this invocation. Clones retain its actual
/// identity; Arc<()> value equality would incorrectly join distinct calls.
#[derive(Debug, Clone)]
pub struct Request {
    identity: Arc<()>,
    incoming: Incoming,
}
impl Request {
    pub fn incoming(&self) -> &Incoming {
        &self.incoming
    }
    pub fn instruction(&self) -> u32 {
        0x604860
    }
    pub fn bytes(&self) -> u32 {
        1240
    }
    pub fn same_call(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity) && self.incoming == other.incoming
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.incoming.validate()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeSite {
    Refcount,
    Derived,
}
impl TreeSite {
    fn index(self) -> usize {
        match self {
            Self::Refcount => 0,
            Self::Derived => 1,
        }
    }
    fn descriptor(self) -> usize {
        match self {
            Self::Refcount => 24,
            Self::Derived => 116,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    RefcountReturned,
    Base72Returned,
    FirstDerivedChildReturned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    AllocateProvider,
    AllocateTree(TreeSite),
    Base72,
    Collection { instruction: u32 },
}

#[derive(Debug, Clone, Copy)]
enum CallerFrame {
    Factory,
    Derived,
    Base,
    Child,
    Base72,
}
impl Frames {
    fn ebp(self, frame: CallerFrame) -> u64 {
        match frame {
            CallerFrame::Factory => self.factory,
            CallerFrame::Derived => self.derived,
            CallerFrame::Base => self.base,
            CallerFrame::Child => self.child,
            CallerFrame::Base72 => self.base72,
        }
    }
}
#[derive(Debug, Clone, Copy)]
enum Value {
    Constant(u32),
    ParentAddress(u32),
    TreeAddress(TreeSite),
}
#[derive(Debug, Clone, Copy)]
enum Op {
    Caller(CallerFrame, i32, Value, u32, bool),
    Parent(usize, Value, u32),
    Tree(TreeSite),
    TreeWord(TreeSite, usize, Value, u32),
    TreeByte(TreeSite, usize, u8, u32),
    Flag(TreeSite),
    Base72Child,
    Cut(Cut),
    Boundary,
}
fn program() -> Vec<Op> {
    use CallerFrame::*;
    use Op::*;
    use Value::*;
    let mut p = vec![
        Caller(Factory, -16, ParentAddress(0), 0x604868, false),
        Caller(Factory, -4, Constant(0), 0x60486f, false),
        Caller(Derived, -16, ParentAddress(0), 0x6048a5, false),
        Parent(0, Constant(0x673738), 0x5d5834),
        Parent(12, Constant(0), 0x5d583a),
        Parent(8, Constant(0x673720), 0x5d583d),
        Caller(Base, -16, ParentAddress(0), 0x5d5844, false),
        Parent(4, Constant(0x673714), 0x5d5847),
        Parent(8, Constant(0x6736fc), 0x5d584e),
        Caller(Base, -4, Constant(0), 0x5d5858, false),
        Parent(16, Constant(0), 0x5d585b),
    ];
    tree_program(&mut p, TreeSite::Refcount);
    p.extend([
        Parent(20, Constant(0x676a6c), 0x602ee2),
        Cut(self::Cut::RefcountReturned),
        Caller(Base, -4, Constant(1), 0x5d5866, true),
        Caller(Base72, 8, Constant(0), 0x5d59c6, false),
        Caller(Child, -20, ParentAddress(36), 0x5d59c8, false),
        Base72Child,
        Cut(self::Cut::Base72Returned),
        Caller(Child, -4, Constant(0), 0x5d59d0, false),
        Parent(108, Constant(0x6737d4), 0x5d59db),
        Parent(112, Constant(0), 0x5d59e2),
    ]);
    tree_program(&mut p, TreeSite::Derived);
    p.extend([
        Parent(36, Constant(0x6737bc), 0x5d59f2),
        Parent(40, Constant(0x6737b0), 0x5d59f8),
        Parent(44, Constant(0x673798), 0x5d59ff),
        Parent(52, Constant(0x673790), 0x5d5a06),
        Parent(108, Constant(0x673768), 0x5d5a0d),
        Cut(self::Cut::FirstDerivedChildReturned),
        Caller(Base, -4, Constant(2), 0x5d5875, true),
        Boundary,
    ]);
    p
}
fn tree_program(p: &mut Vec<Op>, site: TreeSite) {
    let d = site.descriptor();
    p.extend([
        Op::Parent(d, Value::Constant(0), 0x55e376),
        Op::Tree(site),
        Op::Parent(d + 4, Value::Constant(0), 0x55e37e),
        Op::Parent(d, Value::TreeAddress(site), 0x55e387),
        Op::Flag(site),
        Op::TreeByte(site, 0, 0, 0x55e38e),
        Op::TreeWord(site, 4, Value::Constant(0), 0x55e393),
        Op::TreeWord(site, 8, Value::TreeAddress(site), 0x55e399),
        Op::TreeWord(site, 12, Value::TreeAddress(site), 0x55e39e),
    ]);
}

#[derive(Debug, Clone)]
struct Images {
    parent: Owner,
    caller: Owner,
    trees: [Option<Owner>; 2],
    writes: Vec<Write>,
}
fn write(
    owner: &mut Owner,
    offset: usize,
    bytes: &[u8],
    instruction: u32,
    writes: &mut Vec<Write>,
) {
    let before = owner.data[offset..offset + bytes.len()].to_vec();
    owner.data[offset..offset + bytes.len()].copy_from_slice(bytes);
    writes.push(Write {
        owner: owner.token,
        offset,
        instruction,
        child_return: false,
        before,
        after: bytes.to_vec(),
    });
}
impl Images {
    fn new(parent: &Owner, caller: &Owner) -> Self {
        Self {
            parent: parent.clone(),
            caller: caller.clone(),
            trees: [None, None],
            writes: Vec::with_capacity(128),
        }
    }
    fn value(&self, value: Value) -> u32 {
        match value {
            Value::Constant(v) => v,
            Value::ParentAddress(offset) => (self.parent.token + u64::from(offset)) as u32,
            Value::TreeAddress(site) => self.trees[site.index()].as_ref().unwrap().token as u32,
        }
    }
    fn apply(&mut self, op: Op, frames: Frames) -> Result<(), Stop> {
        match op {
            Op::Caller(frame, relative, value, pc, byte) => {
                let offset = (frames
                    .ebp(frame)
                    .checked_add_signed(i64::from(relative))
                    .unwrap()
                    - self.caller.token) as usize;
                let value = self.value(value).to_le_bytes();
                write(
                    &mut self.caller,
                    offset,
                    &value[..if byte { 1 } else { 4 }],
                    pc,
                    &mut self.writes,
                );
            }
            Op::Parent(offset, value, pc) => {
                let value = self.value(value).to_le_bytes();
                write(&mut self.parent, offset, &value, pc, &mut self.writes);
            }
            Op::TreeWord(site, offset, value, pc) => {
                let value = self.value(value).to_le_bytes();
                write(
                    self.trees[site.index()].as_mut().unwrap(),
                    offset,
                    &value,
                    pc,
                    &mut self.writes,
                );
            }
            Op::TreeByte(site, offset, value, pc) => write(
                self.trees[site.index()].as_mut().unwrap(),
                offset,
                &[value],
                pc,
                &mut self.writes,
            ),
            Op::Flag(site) => {
                let address = match site {
                    TreeSite::Refcount => frames.refcount - 2,
                    TreeSite::Derived => frames.child - 14,
                };
                let flag = self.caller.data[(address - self.caller.token) as usize];
                write(
                    &mut self.parent,
                    site.descriptor() + 8,
                    &[flag],
                    0x55e38b,
                    &mut self.writes,
                );
            }
            _ => {
                return Err(unsupported(
                    "system image replay named a non-store operation",
                ));
            }
        }
        Ok(())
    }
}

/// Sealed complete images at an actual source cut, including the allocated
/// preimages needed to replay every preceding effect. Not a completed provider.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    request: Request,
    initial_parent: Owner,
    initial_trees: [Option<Owner>; 2],
    base72: Option<Arc<base72::Prefix>>,
    base72_call: Option<Arc<Base72Call>>,
    position: usize,
    images: Images,
}
impl Checkpoint {
    pub fn source_ordinal(&self) -> usize {
        self.position
    }
    pub fn request(&self) -> &Request {
        &self.request
    }
    pub fn initial_parent(&self) -> &Owner {
        &self.initial_parent
    }
    pub fn parent(&self) -> &Owner {
        &self.images.parent
    }
    pub fn caller(&self) -> &Owner {
        &self.images.caller
    }
    pub fn tree(&self, site: TreeSite) -> Option<&Owner> {
        self.images.trees[site.index()].as_ref()
    }
    pub fn initial_tree(&self, site: TreeSite) -> Option<&Owner> {
        self.initial_trees[site.index()].as_ref()
    }
    pub fn base72(&self) -> Option<&base72::Prefix> {
        self.base72.as_deref()
    }
    pub fn base72_call(&self) -> Option<&Base72Call> {
        self.base72_call.as_deref()
    }
    pub fn writes(&self) -> &[Write] {
        &self.images.writes
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.request.validate()?;
        if self.initial_parent.data.len() != 1240 || self.position > program().len() {
            return Err(unsupported("system source cut has another owner/position"));
        }
        let mut owners = vec![&self.initial_parent, &self.request.incoming.caller];
        for owner in self.initial_trees.iter().flatten() {
            if owner.data.len() != 24 {
                return Err(unsupported(
                    "system tree has another complete allocation extent",
                ));
            }
            owners.push(owner);
        }
        if let Some(prefix) = self.base72.as_deref() {
            prefix.validate()?;
            if let Some(debug) = &prefix.section().debug_info {
                owners.push(debug);
            }
        }
        disjoint(&owners)?;
        let mut expected = Images::new(&self.initial_parent, &self.request.incoming.caller);
        let mut used_trees = [false; 2];
        let mut used_base72 = false;
        for op in program().into_iter().take(self.position) {
            match op {
                Op::Tree(site) => {
                    used_trees[site.index()] = true;
                    expected.trees[site.index()] =
                        Some(self.initial_trees[site.index()].clone().ok_or_else(|| {
                            unsupported("system source cut lost its actual tree return")
                        })?);
                }
                Op::Base72Child => {
                    used_base72 = true;
                    let prefix = self
                        .base72
                        .as_deref()
                        .ok_or_else(|| unsupported("system source cut lost its Base72 child"))?;
                    let call = self.base72_call.as_deref().ok_or_else(|| {
                        unsupported("system Base72 source cut lost its originating factory call")
                    })?;
                    call.validate()?;
                    if !call.checkpoint.request.same_call(&self.request)
                        || !call.request.same_call(prefix.request())
                        || call.checkpoint.initial_parent != self.initial_parent
                    {
                        return Err(unsupported(
                            "system Base72 child lost its originating factory invocation",
                        ));
                    }
                    if prefix.request().initial_parent() != &expected.parent
                        || prefix.request().initial_caller_frame() != &expected.caller
                        || prefix.request().ebp() != self.request.incoming.frames.base72
                    {
                        return Err(unsupported(
                            "system Base72 prefix belongs to another parent/caller cut",
                        ));
                    }
                    expected.parent = prefix.parent().clone();
                    expected.caller = prefix.caller_frame().clone();
                    expected.writes.extend_from_slice(prefix.writes());
                }
                Op::Cut(_) => {}
                Op::Boundary => {
                    return Err(unsupported(
                        "system cursor crossed an unresolved constructor",
                    ));
                }
                op => expected.apply(op, self.request.incoming.frames)?,
            }
        }
        if self
            .initial_trees
            .iter()
            .zip(used_trees)
            .any(|(owner, used)| owner.is_some() != used)
            || self.base72.is_some() != used_base72
            || self.base72_call.is_some() != used_base72
        {
            return Err(unsupported(
                "system source cut retains an unissued future resource",
            ));
        }
        if expected.parent != self.images.parent
            || expected.caller != self.images.caller
            || expected.trees != self.images.trees
            || expected.writes != self.images.writes
        {
            return Err(unsupported(
                "system images or effect history differ from their source cut",
            ));
        }
        Ok(())
    }
}
/// This link is sealed inside the actual factory cursor, rather than inferred
/// from equal Owner bytes. The concrete host must find its issuing factory
/// invocation in the same live heap before first section allocation.
#[derive(Debug, Clone)]
pub struct Base72Call {
    checkpoint: Checkpoint,
    request: base72::Request,
}
impl Base72Call {
    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }
    pub fn request(&self) -> &base72::Request {
        &self.request
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.checkpoint.validate()?;
        self.request.validate()?;
        if !matches!(
            program().get(self.checkpoint.position),
            Some(Op::Base72Child)
        ) || self.request.initial_parent() != self.checkpoint.parent()
            || self.request.initial_caller_frame() != self.checkpoint.caller()
            || self.request.ebp() != self.checkpoint.request.incoming.frames.base72
        {
            return Err(unsupported(
                "system Base72 call differs from its originating factory cut",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct TreeRequest {
    identity: Arc<()>,
    checkpoint: Checkpoint,
    site: TreeSite,
}
impl TreeRequest {
    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }
    pub fn site(&self) -> TreeSite {
        self.site
    }
    pub fn bytes(&self) -> u32 {
        24
    }
    pub fn instruction(&self) -> u32 {
        0x55e379
    }
    pub fn same_call(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
            && self.site == other.site
            && self.checkpoint.request.same_call(&other.checkpoint.request)
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.checkpoint.validate()?;
        if !matches!(program().get(self.checkpoint.position),Some(Op::Tree(site)) if *site==self.site)
        {
            return Err(unsupported(
                "system allocation is not at its source tree call",
            ));
        }
        Ok(())
    }
}

pub trait Host {
    fn allocate_provider(&mut self, request: &Request) -> Result<Allocated, Stop>;
    fn allocate_tree(&mut self, request: &TreeRequest) -> Result<Allocated, Stop>;
    fn prepare_base72(&mut self, checkpoint: &Checkpoint) -> Result<(), Stop>;
    fn initialize_base72(
        &mut self,
        call: &Base72Call,
    ) -> Result<crate::native_application::InitializedSection, Stop>;
    fn acknowledge_base72(
        &mut self,
        call: &Base72Call,
        prefix: &base72::Prefix,
    ) -> Result<(), Stop>;
    fn synchronize_checkpoint(&mut self, checkpoint: &Checkpoint) -> Result<(), Stop>;
}
struct ChildHost<'a, H> {
    host: &'a mut H,
    checkpoint: &'a Checkpoint,
    call: &'a mut Option<Arc<Base72Call>>,
}
impl<H: Host> base72::Host for ChildHost<'_, H> {
    fn initialize_section(
        &mut self,
        request: &base72::Request,
    ) -> Result<crate::native_application::InitializedSection, Stop> {
        if let Some(call) = self.call.as_ref() {
            if !call.request.same_call(request) {
                return Err(unsupported(
                    "system Base72 child changed its pending invocation",
                ));
            }
        } else {
            let call = Base72Call {
                checkpoint: self.checkpoint.clone(),
                request: request.clone(),
            };
            call.validate()?;
            *self.call = Some(Arc::new(call));
        }
        self.host.initialize_base72(self.call.as_ref().unwrap())
    }
}

#[derive(Debug)]
pub struct Construction {
    request: Request,
    pending_parent: Option<Allocated>,
    images: Option<Images>,
    initial_parent: Option<Owner>,
    initial_trees: [Option<Owner>; 2],
    pending_tree: Option<Allocated>,
    tree_request: Option<TreeRequest>,
    child: Option<base72::Construction>,
    pending_prefix: Option<Arc<base72::Prefix>>,
    prefix: Option<Arc<base72::Prefix>>,
    base72_call: Option<Arc<Base72Call>>,
    child_prepared: bool,
    position: usize,
    reserved_history: Option<Vec<Write>>,
}
impl Construction {
    pub fn new(incoming: Incoming) -> Result<Self, BeginError> {
        if let Err(stop) = incoming.validate() {
            return Err(BeginError { stop, incoming });
        }
        let mut history = Vec::new();
        if history.try_reserve_exact(256).is_err() {
            return Err(BeginError {
                stop: Stop::Unavailable("system effect history could not reserve storage".into()),
                incoming,
            });
        }
        Ok(Self {
            request: Request {
                identity: Arc::new(()),
                incoming,
            },
            pending_parent: None,
            images: None,
            initial_parent: None,
            initial_trees: [None, None],
            pending_tree: None,
            tree_request: None,
            child: None,
            pending_prefix: None,
            prefix: None,
            base72_call: None,
            child_prepared: false,
            position: 0,
            reserved_history: Some(history),
        })
    }
    pub fn request(&self) -> &Request {
        &self.request
    }
    pub fn base72_call(&self) -> Option<&Base72Call> {
        self.base72_call.as_deref()
    }
    pub fn base72_prefix(&self) -> Option<&base72::Prefix> {
        self.prefix.as_deref()
    }
    pub fn checkpoint(&self) -> Result<Checkpoint, Stop> {
        let checkpoint = Checkpoint {
            request: self.request.clone(),
            initial_parent: self
                .initial_parent
                .clone()
                .ok_or_else(|| unsupported("system allocation has not returned a parent"))?,
            initial_trees: self.initial_trees.clone(),
            base72: self.prefix.clone(),
            base72_call: if self.prefix.is_some() {
                self.base72_call.clone()
            } else {
                None
            },
            position: self.position,
            images: self.images.clone().unwrap(),
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }
    pub fn pending_call(&self) -> Option<Call> {
        if self.images.is_none() {
            return Some(Call::AllocateProvider);
        }
        match program().get(self.position) {
            Some(Op::Tree(site)) => Some(Call::AllocateTree(*site)),
            Some(Op::Base72Child) => Some(Call::Base72),
            Some(Op::Boundary) => Some(Call::Collection {
                instruction: 0x5d5879,
            }),
            _ => None,
        }
    }
    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        if self.images.is_none() {
            if self.pending_parent.is_none() {
                self.pending_parent = Some(host.allocate_provider(&self.request)?);
                return Ok(None);
            }
            match self.pending_parent.as_ref().unwrap() {
                Allocated::Null=>return Err(Stop::Unavailable("system factory's null return path is not admitted by this owning-child profile".into())),
                Allocated::Bytes{token,data}=>{
                    let owner=Owner{token:*token,data:data.clone()};
                    if owner.data.len()!=1240 {return Err(unsupported("system factory allocation has another extent"));}
                    disjoint(&[&owner,&self.request.incoming.caller])?;
                    let mut images=Images::new(&owner,&self.request.incoming.caller);
                    images.writes=self.reserved_history.take().unwrap();
                    self.images=Some(images);self.initial_parent=Some(owner);
                    self.pending_parent=None;return Ok(None);
                }
            }
        }
        let op = *program()
            .get(self.position)
            .ok_or_else(|| unsupported("system constructor crossed its source boundary"))?;
        match op {
            Op::Tree(site)=>{
                if self.tree_request.is_none() { self.tree_request=Some(TreeRequest{identity:Arc::new(()),checkpoint:self.checkpoint()?,site}); }
                if self.pending_tree.is_none() { self.pending_tree=Some(host.allocate_tree(self.tree_request.as_ref().unwrap())?);return Ok(None); }
                let Allocated::Bytes{token,data}=self.pending_tree.as_ref().unwrap() else {return Err(Stop::Unavailable("system tree allocation returned null before its source dereference".into()));};
                let owner=Owner{token:*token,data:data.clone()};
                if owner.data.len()!=24 {return Err(unsupported("system tree allocation has another extent"));}
                let images=self.images.as_ref().unwrap();let mut retained=vec![&images.parent,&images.caller,&owner];
                for tree in images.trees.iter().flatten(){retained.push(tree);}
                if let Some(debug)=self.prefix.as_ref().and_then(|prefix|prefix.section().debug_info.as_ref()){retained.push(debug);}
                disjoint(&retained)?;
                self.initial_trees[site.index()]=Some(owner.clone());self.images.as_mut().unwrap().trees[site.index()]=Some(owner);
                self.pending_tree=None;self.tree_request=None;
            }
            Op::Base72Child=>{
                if !self.child_prepared {host.prepare_base72(&self.checkpoint()?)?;self.child_prepared=true;}
                if self.child.is_none() && self.pending_prefix.is_none() {
                    let images=self.images.as_ref().unwrap();
                    self.child=Some(base72::Construction::new(base72::Incoming{parent:images.parent.clone(),caller_frame:images.caller.clone(),ebp:self.request.incoming.frames.base72}).map_err(|e|e.stop)?);
                }
                if self.pending_prefix.is_none() {
                    let checkpoint=self.checkpoint()?;
                    let child=self.child.as_mut().unwrap();
                    child.step(&mut ChildHost{host,checkpoint:&checkpoint,call:&mut self.base72_call})?;
                    if !child.returned() {return Ok(None);}
                    let child=self.child.take().unwrap();
                    let prefix=match child.into_prefix() {
                        Ok(prefix)=>prefix,
                        Err(child)=>{
                            self.child=Some(child);
                            return Err(Stop::Unavailable("system Base72 completed prefix is not available yet".into()));
                        }
                    };
                    self.pending_prefix=Some(Arc::new(prefix));
                }
                let prefix=self.pending_prefix.as_ref().unwrap();prefix.validate()?;
                let images=self.images.as_ref().unwrap();
                if let Some(debug)=&prefix.section().debug_info {
                    let mut retained=vec![&images.parent,&images.caller,debug];for tree in images.trees.iter().flatten(){retained.push(tree);}disjoint(&retained)?;
                }
                host.acknowledge_base72(self.base72_call.as_ref().ok_or_else(||unsupported("system Base72 prefix lost its factory origin call"))?,prefix)?;
                let images=self.images.as_mut().unwrap();images.parent=prefix.parent().clone();images.caller=prefix.caller_frame().clone();images.writes.extend_from_slice(prefix.writes());
                self.prefix=self.pending_prefix.take();
            }
            Op::Cut(cut)=>{
                // Publish only source-authenticated complete allocation images.
                host.synchronize_checkpoint(&self.checkpoint()?)?;
                self.position+=1;return Ok(Some(cut));
            }
            Op::Boundary=>return Err(Stop::Unavailable("system provider still requires collection constructor5d5cf6; no global/virtual/Application return is admitted".into())),
            op=>self.images.as_mut().unwrap().apply(op,self.request.incoming.frames)?,
        }
        self.position += 1;
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
