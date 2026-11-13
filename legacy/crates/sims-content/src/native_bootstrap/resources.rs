use super::memory::{Kind, Memory};
use crate::house_startup::saved_arrays::File;
use sims_sim::{
    native_matrix, native_world,
    native_world_children::vector,
    object_catalog::iff::Stream,
    saved_matrix::{self, Allocated, AllocationRequest, Lookup, Stop},
};

/// The source half of saved_matrix::Host, separated from allocation ownership.
/// Implementations must retain their actual stream, IFF cursor and leases.
pub trait ResourceSource {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop>;
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop>;
    fn release(&mut self, lease: u64) -> Result<(), Stop>;
}

fn handle(lease: u64) -> Result<u32, Stop> {
    u32::try_from(lease)
        .ok()
        .filter(|h| *h != 0)
        .ok_or_else(|| {
            Stop::Unsupported("bootstrap resource lease is not a nonzero IFF handle".into())
        })
}

impl<FileStream: Stream> ResourceSource for File<FileStream> {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop> {
        if callback != saved_matrix::NOOP_RESOURCE_CALLBACK {
            return Err(Stop::Unsupported(
                "bootstrap source requires the original no-op resource callback".into(),
            ));
        }
        self.with_iff(|iff, stream| {
            let handle = iff.by_id(tag, id, true, stream)?;
            Ok(if handle == 0 {
                Lookup::Missing {
                    native_status: iff.error,
                }
            } else {
                Lookup::Found {
                    lease: u64::from(handle),
                }
            })
        })
    }
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop> {
        let handle = handle(lease)?;
        self.with_iff(|iff, _| {
            iff.payload(handle)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| "bootstrap IFF source lease is not resident".into())
        })
    }
    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        let handle = handle(lease)?;
        self.with_iff(|iff, _| iff.release_chunk(handle))
    }
}

/// A temporary borrow of the same heap and real resource source. This adapter
/// does not replace either owner or claim that the caller produced a World.
pub struct ArrayHost<'a, Source> {
    memory: &'a mut Memory,
    source: &'a mut Source,
}

impl<'a, Source> ArrayHost<'a, Source> {
    pub fn new(memory: &'a mut Memory, source: &'a mut Source) -> Self {
        Self { memory, source }
    }
}

impl<Source: ResourceSource> saved_matrix::Host for ArrayHost<'_, Source> {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop> {
        self.source.lookup(tag, id, callback)
    }
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop> {
        self.source.source(lease)
    }
    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        self.source.release(lease)
    }
    fn free(&mut self, token: u64) -> Result<(), Stop> {
        self.memory.release_matrix(token)
    }
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        self.memory.allocate_matrix(request)
    }
}

impl<Source: ResourceSource> native_matrix::Host for ArrayHost<'_, Source> {
    fn require_owner(&self, owner: u64, bytes: u32) -> Result<(), Stop> {
        self.memory.require_owner_view(owner, bytes)
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.memory.allocate(Kind::Name, bytes)
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        self.memory.release(token, Kind::Name)
    }
}

impl<Source: ResourceSource> vector::Host for ArrayHost<'_, Source> {
    fn allocate_vector_elements(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        vector::Host::allocate_vector_elements(self.memory, bytes)
    }
}

impl<Source: ResourceSource> native_world::arrays::Host for ArrayHost<'_, Source> {
    fn allocate_array_root(&mut self, _slot: u8, bytes: u32) -> Result<Allocated, Stop> {
        self.memory.allocate(Kind::Owner, bytes)
    }
}
