use sims_sim::{
    native_game, native_world,
    native_system::factory::HeapOrigin,
    native_world::resources::{population, strings},
    native_world_children::{self, Owner, vector},
    saved_matrix::{Allocated, AllocationRequest, Backing, MAX_ALLOCATION_BYTES, Stop},
};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// Allocation addresses repeat between independent heaps. This retained opaque
/// identity distinguishes their lifetimes even after a whole-heap swap.
#[derive(Clone)]
pub(super) struct Identity(HeapOrigin);

// A separate logical heap above the original image/static regions. The strict
// upper bound also keeps every one-past pointer representable as u32.
const START: u64 = 0x1000_0000;
const END: u64 = 0xf000_0000;
const MAX_REQUESTS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Owner,
    Name,
    String,
    Vector,
    ResourceVector,
    Population,
    Matrix {
        columns: u32,
        data_bytes: u32,
    },
    ScopedString {
        scope: StringScope,
        purpose: Purpose,
    },
    Wide(Purpose),
    FileObject,
    ResourceHandle,
    ApplicationTree,
    ApplicationSectionDebug,
    SystemSectionDebug,
    SystemTree,
    SystemList,
    ApplicationMutexWrapper,
    ProgramGlobal,
    Buckets(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StringScope {
    World,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    Construct,
    Grow,
}

struct Region {
    kind: Kind,
    bytes: Vec<u8>,
}

/// Owns initialized Rust allocations in one logical address space. Fresh bytes
/// are zero by this host's explicit allocation policy, not recovered native
/// heap pre-state. There is no import/adopt/deserialize operation. Addresses
/// are never reused, including after release, so stale tokens stay invalid.
pub struct Memory {
    identity: Identity,
    next: u64,
    live_bytes: usize,
    limit: usize,
    issued: usize,
    regions: BTreeMap<u64, Region>,
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

/// Only a recoverable backing-buffer reservation failure may become a supported
/// program allocator's NULL result. Bounds, budgets and unsupported profiles
/// remain host refusals, even when their existing Stop is Unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProgramAllocationError {
    Refused(Stop),
    OutOfMemory,
}

enum AllocationFailure {
    Refused(Stop),
    Reservation(Reservation),
}

#[derive(Clone, Copy)]
enum Reservation {
    Backing,
    ReturnedImage,
}

impl From<Stop> for AllocationFailure {
    fn from(stop: Stop) -> Self {
        Self::Refused(stop)
    }
}

impl AllocationFailure {
    fn into_stop(self) -> Stop {
        match self {
            Self::Refused(stop) => stop,
            Self::Reservation(Reservation::Backing) => {
                Stop::Unavailable("bootstrap heap could not reserve memory".into())
            }
            Self::Reservation(Reservation::ReturnedImage) => {
                Stop::Unavailable("bootstrap allocation copy could not reserve memory".into())
            }
        }
    }

    fn into_program_error(self) -> ProgramAllocationError {
        match self {
            Self::Refused(stop) => ProgramAllocationError::Refused(stop),
            Self::Reservation(_) => ProgramAllocationError::OutOfMemory,
        }
    }
}

impl Memory {
    pub fn new(max_live_bytes: usize) -> Result<Self, Stop> {
        if max_live_bytes == 0 || max_live_bytes > MAX_ALLOCATION_BYTES {
            return Err(unsupported(
                "bootstrap heap limit is outside the bounded profile",
            ));
        }
        Ok(Self {
            identity: Identity(HeapOrigin::new()),
            next: START,
            live_bytes: 0,
            limit: max_live_bytes,
            issued: 0,
            regions: BTreeMap::new(),
        })
    }

    pub fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    pub(super) fn identity(&self) -> Identity {
        self.identity.clone()
    }

    pub(super) fn has_identity(&self, identity: &Identity) -> bool {
        self.identity.0 == identity.0
    }
    /// Retained heap lifetime for first-call constructor provenance.
    pub fn system_origin(&self) -> HeapOrigin { self.identity.0.clone() }

    pub fn live_allocations(&self) -> usize {
        self.regions.len()
    }

    pub(super) fn allocate(&mut self, kind: Kind, bytes: u32) -> Result<Allocated, Stop> {
        self.allocate_using(kind, bytes, Vec::try_reserve_exact)
            .map_err(AllocationFailure::into_stop)
    }

    pub(super) fn allocate_program_global(
        &mut self,
        bytes: u32,
    ) -> Result<Allocated, ProgramAllocationError> {
        self.allocate_program_global_using(bytes, Vec::try_reserve_exact)
    }

    fn allocate_program_global_using(
        &mut self,
        bytes: u32,
        reserve: impl FnMut(&mut Vec<u8>, usize) -> Result<(), std::collections::TryReserveError>,
    ) -> Result<Allocated, ProgramAllocationError> {
        self.allocate_using(Kind::ProgramGlobal, bytes, reserve)
            .map_err(AllocationFailure::into_program_error)
    }

    fn allocate_using(
        &mut self,
        kind: Kind,
        bytes: u32,
        mut reserve: impl FnMut(&mut Vec<u8>, usize) -> Result<(), std::collections::TryReserveError>,
    ) -> Result<Allocated, AllocationFailure> {
        let bytes = bytes as usize;
        let next_live = self
            .live_bytes
            .checked_add(bytes)
            .ok_or_else(|| unsupported("bootstrap heap byte count overflows"))?;
        if bytes == 0 || bytes > MAX_ALLOCATION_BYTES {
            return Err(unsupported("bootstrap allocation extent is unsupported").into());
        }
        if next_live > self.limit || self.issued >= MAX_REQUESTS {
            return Err(Stop::Unavailable(
                "bootstrap heap profile is exhausted".into(),
            ).into());
        }
        let token = self
            .next
            .checked_add(15)
            .map(|n| n & !15)
            .ok_or_else(|| unsupported("bootstrap heap address overflows"))?;
        let end = token
            .checked_add(bytes as u64)
            .filter(|end| *end <= END)
            .ok_or_else(|| Stop::Unavailable("bootstrap logical address space exhausted".into()))?;
        let payload = match kind {
            Kind::Matrix {
                columns,
                data_bytes,
            } => {
                let directory = (columns as usize)
                    .checked_mul(4)
                    .ok_or_else(|| unsupported("matrix directory extent overflows"))?;
                if columns == 0 || directory.checked_add(data_bytes as usize) != Some(bytes) {
                    return Err(unsupported(
                        "matrix allocation request has inconsistent extents",
                    ).into());
                }
                directory
            }
            _ => 0,
        };
        let mut allocation = Vec::new();
        reserve(&mut allocation, bytes)
            .map_err(|_| AllocationFailure::Reservation(Reservation::Backing))?;
        allocation.resize(bytes, 0);
        // saved_matrix returns payload only, while its token names the start of
        // the complete directory+payload allocation. Reserve both in the ledger.
        let mut data = Vec::new();
        reserve(&mut data, allocation.len() - payload)
            .map_err(|_| AllocationFailure::Reservation(Reservation::ReturnedImage))?;
        data.extend_from_slice(&allocation[payload..]);
        self.regions.insert(
            token,
            Region {
                kind,
                bytes: allocation,
            },
        );
        self.next = end;
        self.live_bytes = next_live;
        self.issued += 1;
        Ok(Allocated::Bytes { token, data })
    }

    pub fn allocate_owner(&mut self, bytes: u32) -> Result<Owner, Stop> {
        let Allocated::Bytes { token, data } = self.allocate(Kind::Owner, bytes)? else {
            unreachable!("this allocator never manufactures a null native result")
        };
        Ok(Owner { token, data })
    }

    pub fn allocation_bytes(&self, token: u64) -> Result<&[u8], Stop> {
        self.regions
            .get(&token)
            .map(|r| r.bytes.as_slice())
            .ok_or_else(|| {
                Stop::Unavailable("bootstrap allocation is not live in this heap".into())
            })
    }

    /// Explicitly copy a constructor's current full allocation image back into
    /// this heap. Returned Owner/Allocated images are values, not shared mutable
    /// memory. This operation is synchronization, not source-epoch admission.
    pub fn synchronize_owner(&mut self, owner: &Owner) -> Result<(), Stop> {
        let region = self
            .regions
            .get_mut(&owner.token)
            .ok_or_else(|| Stop::Unavailable("bootstrap owner is not live in this heap".into()))?;
        if matches!(region.kind, Kind::Matrix { .. }) || owner.data.len() != region.bytes.len() {
            return Err(unsupported(
                "bootstrap synchronization needs the complete allocation image",
            ));
        }
        region.bytes.copy_from_slice(&owner.data);
        Ok(())
    }

    /// Materialize the model's relative column offsets as actual logical
    /// pointer words, followed by its modified payload, in the same allocation.
    pub fn synchronize_matrix(&mut self, backing: &Backing) -> Result<(), Stop> {
        let region = self
            .regions
            .get_mut(&backing.token)
            .ok_or_else(|| Stop::Unavailable("matrix backing is not live in this heap".into()))?;
        let Kind::Matrix {
            columns,
            data_bytes,
        } = region.kind
        else {
            return Err(unsupported(
                "matrix synchronization names another allocation kind",
            ));
        };
        if backing.column_offsets.len() != columns as usize
            || backing.data.len() != data_bytes as usize
        {
            return Err(unsupported(
                "matrix synchronization changed its complete allocation shape",
            ));
        }
        let directory = columns as usize * 4;
        let beginning = backing.token + directory as u64;
        let end = beginning + u64::from(data_bytes);
        for offset in &backing.column_offsets {
            let address = beginning + u64::from(*offset);
            if address > end || (data_bytes != 0 && address == end) {
                return Err(unsupported("matrix column points outside its payload"));
            }
        }
        // Validation completes before the first write, including zero-stride
        // matrices whose directory points at the empty payload's end.
        for (index, offset) in backing.column_offsets.iter().enumerate() {
            let address = (beginning + u64::from(*offset)) as u32;
            region.bytes[index * 4..index * 4 + 4].copy_from_slice(&address.to_le_bytes());
        }
        region.bytes[directory..].copy_from_slice(&backing.data);
        Ok(())
    }

    pub fn require_owner_view(&self, token: u64, bytes: u32) -> Result<(), Stop> {
        let end = token
            .checked_add(u64::from(bytes))
            .ok_or_else(|| unsupported("bootstrap owner view overflows"))?;
        if bytes == 0
            || !self
                .regions
                .range(..=token)
                .next_back()
                .is_some_and(|(base, region)| {
                    region.kind == Kind::Owner && end <= *base + region.bytes.len() as u64
                })
        {
            return Err(Stop::Unavailable(
                "bootstrap view has no complete live containing owner".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn release(&mut self, token: u64, kind: Kind) -> Result<(), Stop> {
        let region = self.regions.get(&token).ok_or_else(|| {
            Stop::Unavailable(
                "bootstrap release names an absent or already released allocation".into(),
            )
        })?;
        if region.kind != kind {
            return Err(unsupported(
                "bootstrap release names another allocation kind",
            ));
        }
        self.live_bytes -= region.bytes.len();
        self.regions.remove(&token);
        Ok(())
    }

    pub(super) fn allocation_kind(&self, token: u64) -> Result<Kind, Stop> {
        self.regions
            .get(&token)
            .map(|region| region.kind)
            .ok_or_else(|| {
                Stop::Unavailable("bootstrap allocation is not live in this heap".into())
            })
    }

    pub(super) fn release_owned(&mut self, owner: &Owner, kind: Kind) -> Result<(), Stop> {
        let region = self.regions.get(&owner.token).ok_or_else(|| {
            Stop::Unavailable("bootstrap release has no live complete owner".into())
        })?;
        if owner.data.len() != region.bytes.len() {
            return Err(unsupported(
                "bootstrap release differs from its complete issued extent",
            ));
        }
        self.release(owner.token, kind)
    }

    pub fn release_owner(&mut self, token: u64) -> Result<(), Stop> {
        self.release(token, Kind::Owner)
    }

    pub fn allocate_matrix(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        self.allocate(
            Kind::Matrix {
                columns: request.columns,
                data_bytes: request.data_bytes,
            },
            request.total_bytes,
        )
    }

    pub fn release_matrix(&mut self, token: u64) -> Result<(), Stop> {
        let kind =
            self.regions.get(&token).map(|r| r.kind).ok_or_else(|| {
                Stop::Unavailable("matrix release names an absent allocation".into())
            })?;
        if !matches!(kind, Kind::Matrix { .. }) {
            return Err(unsupported("matrix release names another allocation kind"));
        }
        self.release(token, kind)
    }
}

impl native_game::Host for Memory {
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.allocate(Kind::Name, bytes)
    }
}

impl native_world::Host for Memory {
    fn allocate(&mut self, request: native_world::Request) -> Result<Allocated, Stop> {
        self.allocate(Kind::Owner, request.bytes()?)
    }
}

impl native_world_children::Host for Memory {
    fn allocate(&mut self, request: native_world_children::Request) -> Result<Allocated, Stop> {
        self.allocate(Kind::Owner, request.bytes()?)
    }
}

impl vector::Host for Memory {
    fn allocate_vector_elements(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.allocate(Kind::Vector, bytes)
    }
}

impl strings::Host for Memory {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        self.allocate(Kind::String, request.bytes())
    }
    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        self.release(token, Kind::String)
    }
}

impl native_world::resources::Host for Memory {
    fn allocate_resource_vector(&mut self, vector: u8, bytes: u32) -> Result<Allocated, Stop> {
        if vector >= 7 {
            return Err(unsupported("unknown resource vector allocation"));
        }
        self.allocate(Kind::ResourceVector, bytes)
    }
}

impl population::Host for Memory {
    fn allocate_population(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.allocate(Kind::Population, bytes)
    }
    fn free_null(&mut self) -> Result<(), Stop> {
        // The recovered call explicitly passes null. It owns no allocation.
        Ok(())
    }
}
