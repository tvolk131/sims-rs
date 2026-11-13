//! The bounded32-bit RtlInitializeCriticalSection normal-return layout recovered
//! from the identified DLL. These are logical pointer images, not a Windows
//! thread/heap epoch. Enter/leave, recursion and destruction are separate calls.
use super::{InitializedSection, SectionRequest, disjoint, span, unsupported};
use crate::{native_world_children::Owner, saved_matrix::Stop};

#[cfg(test)]
mod tests;

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn request_valid(request: &SectionRequest) -> Result<(), Stop> {
    span(&request.region)?;
    if request.instruction != 0x5d91bc
        || request.import_slot != 0x666114
        || request.region.data.len() != 24
    {
        return Err(unsupported(
            "section request differs from the recovered application import",
        ));
    }
    Ok(())
}

impl InitializedSection {
    /// Initialize from an actual allocator result while the caller retains its
    /// input images on every error. The source overwrites all32 debug bytes and
    /// all24 section bytes; it also returns normally when that allocation is null.
    pub fn from_debug_info(
        request: &SectionRequest,
        debug_info: Option<&Owner>,
        capability: u64,
    ) -> Result<Self, Stop> {
        request_valid(request)?;
        if capability == 0 {
            return Err(unsupported("section capability must be live and nonzero"));
        }
        if let Some(owner) = debug_info {
            span(owner)?;
            if owner.data.len() != 32 {
                return Err(unsupported("section debug allocation must contain32 bytes"));
            }
            disjoint(vec![span(&request.region)?, span(owner)?])?;
        }
        let debug_info = debug_info.map(|owner| Owner {
            token: owner.token,
            data: words(&[
                0,
                request.region.token as u32,
                owner.token as u32 + 8,
                owner.token as u32 + 8,
                0,
                0,
                0,
                0,
            ]),
        });
        let region = Owner {
            token: request.region.token,
            data: words(&[
                debug_info.as_ref().map_or(0, |owner| owner.token as u32),
                u32::MAX,
                0,
                0,
                0,
                0,
            ]),
        };
        Ok(Self {
            region,
            capability,
            debug_info,
        })
    }

    pub fn validate(&self, request: &SectionRequest) -> Result<(), Stop> {
        request_valid(request)?;
        if self.capability == 0
            || self.region.token != request.region.token
            || self.region.data.len() != 24
        {
            return Err(unsupported(
                "section return has a foreign capability or region",
            ));
        }
        let expected = Self::from_debug_info(request, self.debug_info.as_ref(), self.capability)?;
        if self.region != expected.region || self.debug_info != expected.debug_info {
            return Err(unsupported(
                "section bytes and retained debug allocation differ from initialization",
            ));
        }
        Ok(())
    }
}
