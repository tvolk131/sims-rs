use serde::{Deserialize, Serialize};
use sims_sim::{
    object_catalog::iff::{Iff, Stream},
    saved_matrix::{self, Allocated, AllocationRequest, Lookup, Stop},
    saved_world::{self, Legacy, Temporary},
};

mod continuation;
pub use continuation::{BeginError, Inputs, ReadOwner, Returned};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File<FileStream> {
    iff: Iff,
    stream: FileStream,
    #[serde(deserialize_with = "retained_fault")]
    fault: Option<String>,
}

impl<FileStream> File<FileStream> {
    pub fn from_returned(iff: Iff, stream: FileStream) -> Self {
        Self {
            iff,
            stream,
            fault: None,
        }
    }

    pub fn iff(&self) -> &Iff {
        &self.iff
    }

    pub fn stream(&self) -> &FileStream {
        &self.stream
    }

    fn ready(&self) -> Result<(), Stop> {
        if let Some(message) = &self.fault {
            return Err(Stop::Unavailable(message.clone()));
        }
        Ok(())
    }

    pub fn with_iff<Value>(
        &mut self,
        operation: impl FnOnce(&mut Iff, &mut FileStream) -> Result<Value, String>,
    ) -> Result<Value, Stop> {
        self.ready()?;
        match operation(&mut self.iff, &mut self.stream) {
            Ok(value) => Ok(value),
            Err(message) => {
                let message =
                    format!("saved-array IFF operation has no returned continuation: {message}");
                self.fault = Some(message.clone());
                Err(Stop::Unavailable(message))
            }
        }
    }
}

fn retained_fault<'input, Input: serde::Deserializer<'input>>(
    input: Input,
) -> Result<Option<String>, Input::Error> {
    Option::deserialize(input)
}

pub struct Source<'owner, FileStream, Services> {
    pub file: &'owner mut File<FileStream>,
    pub services: &'owner mut Services,
}

impl<FileStream: Stream, Services: saved_world::Host> saved_matrix::Host
    for Source<'_, FileStream, Services>
{
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop> {
        if callback != saved_matrix::NOOP_RESOURCE_CALLBACK {
            return Err(Stop::Unsupported(
                "saved-array source requires the original no-op resource callback".into(),
            ));
        }
        self.file.with_iff(|iff, stream| {
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
        self.file.ready()?;
        self.file
            .iff
            .payload(handle(lease)?)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| Stop::Unavailable("saved-array resident source is absent".into()))
    }

    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        let handle = handle(lease)?;
        self.file.with_iff(|iff, _| iff.release_chunk(handle))
    }

    fn free(&mut self, token: u64) -> Result<(), Stop> {
        self.services.free(token)
    }

    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        self.services.allocate(request)
    }
}

impl<FileStream: Stream, Services: saved_world::Host> saved_world::Host
    for Source<'_, FileStream, Services>
{
    fn construct_temporary(&mut self, kind: Legacy) -> Result<Temporary, Stop> {
        self.services.construct_temporary(kind)
    }

    fn destroy_temporary(&mut self, temporary: &Temporary) -> Result<(), Stop> {
        self.services.destroy_temporary(temporary)
    }

    fn name_arrays(&mut self) -> Result<(), Stop> {
        self.services.name_arrays()
    }
}

impl<FileStream: Stream, Services: saved_world::Host + sims_sim::native_matrix::Host>
    sims_sim::native_matrix::Host for Source<'_, FileStream, Services>
{
    fn require_owner(&self, owner: u64, bytes: u32) -> Result<(), Stop> {
        self.services.require_owner(owner, bytes)
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.services.allocate_name(bytes)
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        self.services.release_name(token)
    }
}

fn handle(lease: u64) -> Result<u32, Stop> {
    lease.try_into().map_err(|_| {
        Stop::Unsupported("saved-array lease exceeds the retained IFF handle width".into())
    })
}

#[cfg(test)]
mod tests;
