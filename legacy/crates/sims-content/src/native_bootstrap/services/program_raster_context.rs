//! Owning software raster contexts for the explicit PrivateRasterV1 policy.
//!
//! Parent integration must declare this module and the real `program_raster`
//! sibling. This is not CreateCompatibleDC, an HDC, GDI32 publication, a current
//! screen binding, or qualification of Windows defaults/font/ICM/palette/region
//! behavior. NULL creation refuses. White brush, zeroed software backing and
//! confinement to the actual creator Rust thread are this port's policies.
//!
//! Each published lease owns a real selection and a distinct retained 1x1 mono
//! default. Selected caller bitmaps remain borrowed. Weak observations cannot
//! be converted into owners; authority requires the actual issuing allocation.
//! Registry owns the device; leases have only weak registry links. Cleanup may
//! run on any Rust thread and uses ordinary Rust drops, without native calls.
//! B's bounded history includes failed allocation attempts once A effects may
//! begin; no partial A issuance is hidden by rolling counters back. All source
//! controls are authored UNRUN. Integration and execution belong to the parent.

use crate::native_bootstrap::memory::{Identity, Memory};
use super::program_raster::{
    self, Bitmap, BitmapInfo, BitmapObservation, Device, Pixel, PixelFormat,
    Rect, RgbaImage, Selection,
};
use sims_sim::saved_matrix::Stop;
use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard, Weak},
    thread::{self, ThreadId},
};

#[cfg(test)]
mod tests;

/// Enabling the private layout is explicit; there is deliberately no Default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Disabled,
    PrivateRasterV1,
}

/// Refused construction returns the same owning device, never a replacement.
#[derive(Debug)]
pub struct BeginError {
    pub error: Stop,
    pub device: Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterState {
    pub brush: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub contexts_issued: usize,
    pub live_contexts: usize,
    pub device: program_raster::Observation,
}

// Private allocation identities, not numeric IDs or transferable observations.
struct Issuer {
    _private: (),
}

struct LeaseIdentity {
    issuer: Weak<Issuer>,
    registry: Weak<Mutex<Inner>>,
    slot: usize,
}

/// Non-Clone, non-serializable owner of one issued context lifetime.
pub struct ContextLease {
    identity: Arc<LeaseIdentity>,
}

/// Historical weak identity only. There is no owning upgrade operation.
#[derive(Clone)]
pub struct ContextObservation {
    issuer: Weak<Issuer>,
    identity: Weak<LeaseIdentity>,
    slot: usize,
}

// Declaration order is intentional: detach the Selection before releasing the
// context's default Bitmap, on every success/failure/close/drop cleanup path.
struct Resources {
    selection: Selection,
    default: Bitmap,
    state: RasterState,
}

struct Record {
    identity: Weak<LeaseIdentity>,
    resources: Option<Resources>,
}

struct Inner {
    device: Device,
    records: Vec<Record>,
    live: bool,
}

pub struct Registry {
    memory: Identity,
    creator: ThreadId,
    issuer: Arc<Issuer>,
    inner: Arc<Mutex<Inner>>,
    max_contexts: usize,
}

impl fmt::Debug for ContextLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextLease").finish_non_exhaustive()
    }
}

impl fmt::Debug for ContextObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextObservation").finish_non_exhaustive()
    }
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry").finish_non_exhaustive()
    }
}

fn refused(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn unavailable(message: &str) -> Stop {
    Stop::Unavailable(message.into())
}

impl Registry {
    pub fn new(
        memory: &Memory,
        device: Device,
        policy: Policy,
        max_contexts: usize,
    ) -> Result<Self, BeginError> {
        // These refusals must not even inspect the supplied device.
        if policy != Policy::PrivateRasterV1 {
            return Err(BeginError {
                error: refused("private raster context policy is disabled"),
                device,
            });
        }
        if !(1..=4096).contains(&max_contexts) {
            return Err(BeginError {
                error: refused("private raster context history limit is unsupported"),
                device,
            });
        }
        // Bind actual heap lifetime and creator thread before device access.
        let memory = memory.identity();
        let creator = thread::current().id();
        if let Err(error) = device.snapshot() {
            return Err(BeginError { error, device });
        }
        Ok(Self {
            memory,
            creator,
            issuer: Arc::new(Issuer { _private: () }),
            inner: Arc::new(Mutex::new(Inner {
                device,
                records: Vec::new(),
                live: true,
            })),
            max_contexts,
        })
    }

    // Every operational entry point calls this before locking, reading a
    // context, or accessing the device. Heap refusal precedes thread refusal.
    fn gate(&self, memory: &Memory) -> Result<(), Stop> {
        if !memory.has_identity(&self.memory) {
            return Err(refused("private raster context belongs to another heap"));
        }
        if thread::current().id() != self.creator {
            return Err(refused("private raster context requires its creator thread"));
        }
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, Stop> {
        let inner = self.inner.lock()
            .map_err(|_| unavailable("private raster context registry is poisoned"))?;
        if !inner.live {
            return Err(refused("private raster context registry is closed"));
        }
        Ok(inner)
    }

    fn context_index(&self, inner: &Inner, context: &ContextLease) -> Result<usize, Stop> {
        let identity = &context.identity;
        if !Weak::ptr_eq(&identity.issuer, &Arc::downgrade(&self.issuer))
            || !Weak::ptr_eq(&identity.registry, &Arc::downgrade(&self.inner))
        {
            return Err(refused("private raster context lease is foreign"));
        }
        let record = inner.records.get(identity.slot)
            .ok_or_else(|| refused("private raster context lease was never issued here"))?;
        if !Weak::ptr_eq(&record.identity, &Arc::downgrade(identity))
            || record.resources.is_none()
        {
            return Err(refused("private raster context lease is closed or stale"));
        }
        Ok(identity.slot)
    }

    // All recoverable B metadata reservation and identity creation precede A
    // effects. Once committed, a history slot is never removed or reused, even
    // when a later A allocation/selection refuses and no lease is published.
    fn issue(&self, inner: &mut Inner, source: Option<&Bitmap>) -> Result<ContextLease, Stop> {
        if inner.records.len() >= self.max_contexts {
            return Err(unavailable("private raster context history is exhausted"));
        }
        inner.records.try_reserve(1)
            .map_err(|_| unavailable("private raster context history reservation failed"))?;
        let identity = Arc::new(LeaseIdentity {
            issuer: Arc::downgrade(&self.issuer),
            registry: Arc::downgrade(&self.inner),
            slot: inner.records.len(),
        });
        inner.records.push(Record {
            identity: Arc::downgrade(&identity),
            resources: None,
        });
        let selection = inner.device.new_selection()?;
        let default = inner.device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1)?;
        let mut resources = Resources {
            selection,
            default,
            state: RasterState { brush: [255, 255, 255, 255] },
        };
        inner.device.select(&mut resources.selection, &resources.default)?;
        if let Some(bitmap) = source {
            inner.device.select(&mut resources.selection, bitmap)?;
        }
        inner.records[identity.slot].resources = Some(resources);
        Ok(ContextLease { identity })
    }

    pub fn attach_source(&mut self, memory: &Memory, bitmap: &Bitmap) -> Result<ContextLease, Stop> {
        self.gate(memory)?;
        let mut inner = self.lock()?;
        // Authenticates the actual owner before B history or any A allocations.
        inner.device.describe_bitmap(bitmap)?;
        self.issue(&mut inner, Some(bitmap))
    }

    pub fn create_compatible(
        &mut self,
        memory: &Memory,
        source: Option<&ContextLease>,
    ) -> Result<ContextLease, Stop> {
        self.gate(memory)?;
        let source = source.ok_or_else(|| {
            refused("NULL raster source has no authenticated current-screen binding")
        })?;
        let mut inner = self.lock()?;
        self.context_index(&inner, source)?;
        // Neither source selection nor brush is copied or retained.
        self.issue(&mut inner, None)
    }

    pub fn select_bitmap(
        &mut self,
        memory: &Memory,
        context: &ContextLease,
        bitmap: &Bitmap,
    ) -> Result<BitmapObservation, Stop> {
        self.gate(memory)?;
        let mut inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let Inner { device, records, .. } = &mut *inner;
        let resources = records[index].resources.as_mut()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        device.select(&mut resources.selection, bitmap)?
            .ok_or_else(|| refused("private raster context lost its selected backing"))
    }

    pub fn restore_default(
        &mut self,
        memory: &Memory,
        context: &ContextLease,
    ) -> Result<BitmapObservation, Stop> {
        self.gate(memory)?;
        let mut inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let Inner { device, records, .. } = &mut *inner;
        let resources = records[index].resources.as_mut()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        device.select(&mut resources.selection, &resources.default)?
            .ok_or_else(|| refused("private raster context lost its selected backing"))
    }

    pub fn selected_bitmap(
        &self,
        memory: &Memory,
        context: &ContextLease,
    ) -> Result<BitmapObservation, Stop> {
        self.gate(memory)?;
        let inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let resources = inner.records[index].resources.as_ref()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        inner.device.selected(&resources.selection)?
            .ok_or_else(|| refused("private raster context lost its selected backing"))
    }

    pub fn bitmap_info(&self, memory: &Memory, bitmap: &Bitmap) -> Result<BitmapInfo, Stop> {
        self.gate(memory)?;
        self.lock()?.device.describe_bitmap(bitmap)
    }

    pub fn set_brush(
        &mut self,
        memory: &Memory,
        context: &ContextLease,
        rgba: [u8; 4],
    ) -> Result<(), Stop> {
        self.gate(memory)?;
        let mut inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let resources = inner.records[index].resources.as_mut()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        resources.state.brush = rgba;
        Ok(())
    }

    pub fn state(&self, memory: &Memory, context: &ContextLease) -> Result<RasterState, Stop> {
        self.gate(memory)?;
        let inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let resources = inner.records[index].resources.as_ref()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        Ok(resources.state)
    }

    pub fn fill_rect(
        &mut self,
        memory: &Memory,
        context: &ContextLease,
        rect: Rect,
    ) -> Result<(), Stop> {
        self.gate(memory)?;
        let mut inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let Inner { device, records, .. } = &mut *inner;
        let resources = records[index].resources.as_ref()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        let info = device.selected_info(&resources.selection)?
            .ok_or_else(|| refused("private raster context lost its selected backing"))?;
        let pixel = match (info.format, resources.state.brush) {
            (PixelFormat::Rgba8Straight, rgba) => Pixel::Rgba(rgba),
            (PixelFormat::Mono1Msb16, [0, 0, 0, 255]) => Pixel::Mono(false),
            (PixelFormat::Mono1Msb16, [255, 255, 255, 255]) => Pixel::Mono(true),
            (PixelFormat::Mono1Msb16, _) => {
                return Err(refused("private mono raster brush must be opaque black or white"));
            }
        };
        device.fill_rect(&resources.selection, rect, pixel)
    }

    pub fn readback_rgba(
        &self,
        memory: &Memory,
        context: &ContextLease,
    ) -> Result<RgbaImage, Stop> {
        self.gate(memory)?;
        let inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let resources = inner.records[index].resources.as_ref()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        inner.device.readback_rgba(&resources.selection)
    }

    pub fn observe(
        &self,
        memory: &Memory,
        context: &ContextLease,
    ) -> Result<ContextObservation, Stop> {
        self.gate(memory)?;
        let inner = self.lock()?;
        let slot = self.context_index(&inner, context)?;
        Ok(ContextObservation {
            issuer: Arc::downgrade(&self.issuer),
            identity: Arc::downgrade(&context.identity),
            slot,
        })
    }

    pub fn observation_live(
        &self,
        memory: &Memory,
        observation: &ContextObservation,
    ) -> Result<bool, Stop> {
        self.gate(memory)?;
        if !Weak::ptr_eq(&observation.issuer, &Arc::downgrade(&self.issuer)) {
            return Err(refused("private raster context observation is foreign"));
        }
        let inner = self.lock()?;
        let record = inner.records.get(observation.slot)
            .ok_or_else(|| refused("private raster context observation was never issued here"))?;
        if !Weak::ptr_eq(&record.identity, &observation.identity) {
            return Err(refused("private raster context observation is stale"));
        }
        Ok(record.resources.is_some() && observation.identity.strong_count() != 0)
    }

    pub fn close(&mut self, memory: &Memory, context: &ContextLease) -> Result<(), Stop> {
        self.gate(memory)?;
        let mut inner = self.lock()?;
        let index = self.context_index(&inner, context)?;
        let Inner { device, records, .. } = &mut *inner;
        let resources = records[index].resources.as_mut()
            .ok_or_else(|| refused("private raster context has no live resources"))?;
        // A refusal leaves this lease live; success detaches before releasing
        // the default. Taking the record then revokes all observations once.
        device.clear(&mut resources.selection)?;
        drop(records[index].resources.take());
        Ok(())
    }

    pub fn snapshot(&self, memory: &Memory) -> Result<Observation, Stop> {
        self.gate(memory)?;
        let inner = self.lock()?;
        Ok(Observation {
            contexts_issued: inner.records.len(),
            live_contexts: inner.records.iter().filter(|r| r.resources.is_some()).count(),
            device: inner.device.snapshot()?,
        })
    }
}

impl Drop for ContextLease {
    fn drop(&mut self) {
        let Some(registry) = self.identity.registry.upgrade() else { return };
        let resources = {
            // Poison never admits an operation or clears the poison flag.
            // Recovering a guard here is exclusively for infallible teardown,
            // not retry, healthy-state adoption or a fabricated success value.
            let mut inner = match registry.lock() {
                Ok(inner) => inner,
                Err(poison) => poison.into_inner(),
            };
            inner.records.get_mut(self.identity.slot).and_then(|record| {
                if Weak::ptr_eq(&record.identity, &Arc::downgrade(&self.identity)) {
                    record.resources.take()
                } else {
                    None
                }
            })
        };
        drop(resources);
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poison) => poison.into_inner(),
        };
        inner.live = false;
        for record in &mut inner.records {
            drop(record.resources.take());
        }
        // Device subsequently drops with Inner. No strong context/observation
        // link can keep a registry operational or recreate its issuer epoch.
    }
}
