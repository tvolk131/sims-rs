//! Actual bounded software pixels for the explicit PrivateRasterV1 profile.
//!
//! Parent declares this sibling and wires its exact API to program_raster_context.
//! This is not an HDC, Windows bitmap/defaults, current-screen authority or a GDI
//! export. Fresh zeroed storage, top-down rows and straight RGBA are this port's
//! policies. No native graphics, caller words, callbacks or observation adoption.
//!
//! One mutex serializes publication, drawing, readback and cross-thread Drop.
//! Histories retain distinct allocation identities, never recyclable IDs. Pixel
//! bytes live in those histories only while a caller owner or selection needs
//! them. An observation is weak and cannot promote itself to a public owner.

use sims_sim::saved_matrix::Stop;
use std::{
    fmt,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

const MAX_DIMENSION: u32 = 8192;
const MAX_BITMAP_BYTES: usize = 64 * 1024 * 1024;
const MAX_LIVE_BYTES: usize = 128 * 1024 * 1024;
const MAX_HISTORY: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Mono1Msb16,
    Rgba8Straight,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_bitmap_bytes: usize,
    pub max_live_bytes: usize,
    pub max_bitmaps: usize,
    pub max_selections: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pixel {
    Mono(bool),
    Rgba([u8; 4]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapInfo {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
}

/// An output copy of pixels, with no authority to create/adopt a resource.
#[derive(Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub bitmap_issuances: usize,
    pub selection_issuances: usize,
    pub live_bitmap_bytes: usize,
}

// Indices locate records only. Admission also compares the issuing Shared and
// this actual allocation with the corresponding history record's allocation.
struct BitmapIdentity { index: usize }
struct SelectionIdentity { index: usize }

struct BitmapRecord {
    identity: Arc<BitmapIdentity>,
    info: BitmapInfo,
    handle_live: bool,
    selected_by: Option<usize>,
    pixels: Option<Vec<u8>>,
}

struct SelectionRecord {
    identity: Arc<SelectionIdentity>,
    live: bool,
    selected: Option<usize>,
}

struct State {
    limits: Limits,
    bitmaps: Vec<BitmapRecord>,
    selections: Vec<SelectionRecord>,
}

struct Shared {
    device_live: AtomicBool,
    state: Mutex<State>,
}

/// Sole device owner; moving it preserves its private issuing allocation.
pub struct Device { shared: Arc<Shared> }

/// Non-Clone caller owner. Dropping it revokes observations even if selected.
pub struct Bitmap {
    shared: Arc<Shared>,
    identity: Arc<BitmapIdentity>,
}

/// Non-Clone exclusive selection owner, retaining the actual selected backing.
pub struct Selection {
    shared: Arc<Shared>,
    identity: Arc<SelectionIdentity>,
}

/// Historical identity only. No owning upgrade or adoption method exists.
#[derive(Clone)]
pub struct BitmapObservation {
    shared: Weak<Shared>,
    identity: Weak<BitmapIdentity>,
}

impl BitmapObservation {
    pub fn same_bitmap(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.shared, &other.shared)
            && Weak::ptr_eq(&self.identity, &other.identity)
    }
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("live", &self.shared.device_live.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bitmap").finish_non_exhaustive()
    }
}

impl fmt::Debug for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Selection").finish_non_exhaustive()
    }
}

impl fmt::Debug for BitmapObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitmapObservation").finish_non_exhaustive()
    }
}

fn unsupported(message: &str) -> Stop { Stop::Unsupported(message.into()) }
fn unavailable(message: &str) -> Stop { Stop::Unavailable(message.into()) }

fn host_size(value: u32) -> Result<usize, Stop> {
    usize::try_from(value).map_err(|_| unsupported("raster extent exceeds host size"))
}

fn layout(limits: Limits, format: PixelFormat, width: u32, height: u32)
    -> Result<(BitmapInfo, usize), Stop>
{
    if width == 0 || height == 0 || width > limits.max_width || height > limits.max_height {
        return Err(unsupported("raster dimensions exceed the nonempty configured bound"));
    }
    let w = host_size(width)?;
    let h = host_size(height)?;
    let stride = match format {
        PixelFormat::Mono1Msb16 => w.checked_add(15)
            .and_then(|n| (n / 16).checked_mul(2)),
        PixelFormat::Rgba8Straight => w.checked_mul(4),
    }.ok_or_else(|| unsupported("raster stride overflow"))?;
    let bytes = stride.checked_mul(h)
        .ok_or_else(|| unsupported("raster storage extent overflow"))?;
    if bytes > limits.max_bitmap_bytes {
        return Err(unsupported("raster bitmap storage exceeds its byte bound"));
    }
    Ok((BitmapInfo { width, height, stride, format }, bytes))
}

impl State {
    fn live_bytes(&self) -> Result<usize, Stop> {
        self.bitmaps.iter().filter_map(|b| b.pixels.as_ref()).try_fold(0usize, |n, p| {
            n.checked_add(p.capacity()).ok_or_else(|| unavailable("raster retained-byte overflow"))
        })
    }

    fn bitmap(&self, shared: &Arc<Shared>, bitmap: &Bitmap) -> Result<usize, Stop> {
        if !Arc::ptr_eq(shared, &bitmap.shared) {
            return Err(unsupported("bitmap belongs to another raster device"));
        }
        let index = bitmap.identity.index;
        let record = self.bitmaps.get(index)
            .ok_or_else(|| unsupported("bitmap issuance is absent"))?;
        if !Arc::ptr_eq(&record.identity, &bitmap.identity)
            || !record.handle_live || record.pixels.is_none()
        {
            return Err(unsupported("bitmap owner is revoked"));
        }
        Ok(index)
    }

    fn selection(&self, shared: &Arc<Shared>, selection: &Selection) -> Result<usize, Stop> {
        if !Arc::ptr_eq(shared, &selection.shared) {
            return Err(unsupported("selection belongs to another raster device"));
        }
        let index = selection.identity.index;
        let record = self.selections.get(index)
            .ok_or_else(|| unsupported("selection issuance is absent"))?;
        if !Arc::ptr_eq(&record.identity, &selection.identity) || !record.live {
            return Err(unsupported("selection owner is revoked"));
        }
        Ok(index)
    }

    fn selected_index(&self, selection: usize) -> Result<Option<usize>, Stop> {
        let record = &self.selections[selection];
        if let Some(index) = record.selected {
            let bitmap = self.bitmaps.get(index)
                .ok_or_else(|| unavailable("raster selected backing is absent"))?;
            if bitmap.selected_by != Some(selection) || bitmap.pixels.is_none() {
                return Err(unavailable("raster selected backing is inconsistent"));
            }
        }
        Ok(record.selected)
    }

    fn observation(&self, shared: &Arc<Shared>, index: usize) -> BitmapObservation {
        BitmapObservation {
            shared: Arc::downgrade(shared),
            identity: Arc::downgrade(&self.bitmaps[index].identity),
        }
    }

    // Cleanup has no user code, locks, allocation, or fallible arithmetic. In a
    // poisoned state Drop may only revoke/detach; it never restores device use.
    fn revoke_bitmap(&mut self, identity: &Arc<BitmapIdentity>) {
        if let Some(record) = self.bitmaps.get_mut(identity.index) {
            if Arc::ptr_eq(&record.identity, identity) {
                record.handle_live = false;
                if record.selected_by.is_none() { record.pixels = None; }
            }
        }
    }

    fn detach(&mut self, selection: usize) {
        if let Some(index) = self.selections[selection].selected.take() {
            if let Some(record) = self.bitmaps.get_mut(index) {
                if record.selected_by == Some(selection) {
                    record.selected_by = None;
                    if !record.handle_live { record.pixels = None; }
                }
            }
        }
    }

    fn revoke_selection(&mut self, identity: &Arc<SelectionIdentity>) {
        if let Some(record) = self.selections.get_mut(identity.index) {
            if Arc::ptr_eq(&record.identity, identity) {
                record.live = false;
                self.detach(identity.index);
            }
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.shared.device_live.store(false, Ordering::Release);
        // Outstanding owners retain Shared and their genuine backing. They
        // cannot use it without the sole Device, and ordinary Drop frees it.
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.revoke_bitmap(&self.identity);
    }
}

impl Drop for Selection {
    fn drop(&mut self) {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.revoke_selection(&self.identity);
    }
}

impl Device {
    pub fn new(limits: Limits) -> Result<Self, Stop> {
        if !(1..=MAX_DIMENSION).contains(&limits.max_width)
            || !(1..=MAX_DIMENSION).contains(&limits.max_height)
            || !(1..=MAX_BITMAP_BYTES).contains(&limits.max_bitmap_bytes)
            || !(1..=MAX_LIVE_BYTES).contains(&limits.max_live_bytes)
            || limits.max_live_bytes < limits.max_bitmap_bytes
            || !(1..=MAX_HISTORY).contains(&limits.max_bitmaps)
            || !(1..=MAX_HISTORY).contains(&limits.max_selections)
        {
            return Err(unsupported("raster limits are outside PrivateRasterV1"));
        }
        Ok(Self { shared: Arc::new(Shared {
            device_live: AtomicBool::new(true),
            state: Mutex::new(State { limits, bitmaps: Vec::new(), selections: Vec::new() }),
        }) })
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, Stop> {
        if !self.shared.device_live.load(Ordering::Acquire) {
            return Err(unsupported("raster device is revoked"));
        }
        self.shared.state.lock().map_err(|_| unavailable("raster device state is poisoned"))
    }

    pub fn create_bitmap(&mut self, format: PixelFormat, width: u32, height: u32)
        -> Result<Bitmap, Stop>
    {
        let mut state = self.lock()?;
        let (info, bytes) = layout(state.limits, format, width, height)?;
        if state.bitmaps.len() >= state.limits.max_bitmaps {
            return Err(unsupported("raster bitmap issuance history is exhausted"));
        }
        let live = state.live_bytes()?.checked_add(bytes)
            .ok_or_else(|| unsupported("raster live-byte extent overflow"))?;
        if live > state.limits.max_live_bytes {
            return Err(unsupported("raster retained pixel budget is exhausted"));
        }
        state.bitmaps.try_reserve(1)
            .map_err(|_| unavailable("cannot reserve raster bitmap metadata"))?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(bytes)
            .map_err(|_| unavailable("cannot reserve raster pixel backing"))?;
        // Exact reservation may still overallocate on a host. Charge retained
        // capacity, not just initialized length, before publishing this owner.
        let allocated_live = state.live_bytes()?.checked_add(pixels.capacity())
            .ok_or_else(|| unsupported("raster allocated live-byte extent overflow"))?;
        if pixels.capacity() > state.limits.max_bitmap_bytes
            || allocated_live > state.limits.max_live_bytes
        {
            return Err(unsupported("host raster allocation exceeds the retained pixel budget"));
        }
        pixels.resize(bytes, 0);
        let identity = Arc::new(BitmapIdentity { index: state.bitmaps.len() });
        state.bitmaps.push(BitmapRecord {
            identity: Arc::clone(&identity), info, handle_live: true,
            selected_by: None, pixels: Some(pixels),
        });
        Ok(Bitmap { shared: Arc::clone(&self.shared), identity })
    }

    pub fn describe_bitmap(&self, bitmap: &Bitmap) -> Result<BitmapInfo, Stop> {
        let state = self.lock()?;
        let index = state.bitmap(&self.shared, bitmap)?;
        Ok(state.bitmaps[index].info)
    }

    pub fn new_selection(&mut self) -> Result<Selection, Stop> {
        let mut state = self.lock()?;
        if state.selections.len() >= state.limits.max_selections {
            return Err(unsupported("raster selection issuance history is exhausted"));
        }
        state.selections.try_reserve(1)
            .map_err(|_| unavailable("cannot reserve raster selection metadata"))?;
        let identity = Arc::new(SelectionIdentity { index: state.selections.len() });
        state.selections.push(SelectionRecord {
            identity: Arc::clone(&identity), live: true, selected: None,
        });
        Ok(Selection { shared: Arc::clone(&self.shared), identity })
    }

    pub fn select(&mut self, selection: &mut Selection, bitmap: &Bitmap)
        -> Result<Option<BitmapObservation>, Stop>
    {
        let mut state = self.lock()?;
        let selected_by = state.selection(&self.shared, selection)?;
        let index = state.bitmap(&self.shared, bitmap)?;
        let old = state.selected_index(selected_by)?;
        if let Some(owner) = state.bitmaps[index].selected_by {
            if owner != selected_by {
                return Err(unsupported("bitmap is already selected by another live selection"));
            }
        }
        let previous = old.map(|i| state.observation(&self.shared, i));
        if old == Some(index) { return Ok(previous); }
        // Everything that can refuse has completed before old backing detaches.
        state.detach(selected_by);
        state.bitmaps[index].selected_by = Some(selected_by);
        state.selections[selected_by].selected = Some(index);
        Ok(previous)
    }

    pub fn selected(&self, selection: &Selection) -> Result<Option<BitmapObservation>, Stop> {
        let state = self.lock()?;
        let index = state.selection(&self.shared, selection)?;
        Ok(state.selected_index(index)?.map(|i| state.observation(&self.shared, i)))
    }

    pub fn selected_info(&self, selection: &Selection) -> Result<Option<BitmapInfo>, Stop> {
        let state = self.lock()?;
        let index = state.selection(&self.shared, selection)?;
        Ok(state.selected_index(index)?.map(|i| state.bitmaps[i].info))
    }

    pub fn clear(&mut self, selection: &mut Selection) -> Result<Option<BitmapObservation>, Stop> {
        let mut state = self.lock()?;
        let index = state.selection(&self.shared, selection)?;
        let old = state.selected_index(index)?.map(|i| state.observation(&self.shared, i));
        state.detach(index);
        Ok(old)
    }

    pub fn close_bitmap(&mut self, bitmap: &Bitmap) -> Result<(), Stop> {
        let mut state = self.lock()?;
        let index = state.bitmap(&self.shared, bitmap)?;
        if state.bitmaps[index].selected_by.is_some() {
            return Err(unsupported("cannot close a selected bitmap owner"));
        }
        state.revoke_bitmap(&bitmap.identity);
        Ok(())
    }

    pub fn fill_rect(&mut self, selection: &Selection, rect: Rect, pixel: Pixel) -> Result<(), Stop> {
        let mut state = self.lock()?;
        let selection_index = state.selection(&self.shared, selection)?;
        let index = state.selected_index(selection_index)?
            .ok_or_else(|| unsupported("raster selection has no bitmap"))?;
        let info = state.bitmaps[index].info;
        let right = rect.x.checked_add(rect.width)
            .ok_or_else(|| unsupported("raster rectangle horizontal extent overflow"))?;
        let bottom = rect.y.checked_add(rect.height)
            .ok_or_else(|| unsupported("raster rectangle vertical extent overflow"))?;
        if rect.width == 0 || rect.height == 0 || right > info.width || bottom > info.height {
            return Err(unsupported("raster rectangle must be nonempty and fully in bounds"));
        }
        if !matches!((info.format, pixel),
            (PixelFormat::Mono1Msb16, Pixel::Mono(_))
                | (PixelFormat::Rgba8Straight, Pixel::Rgba(_)))
        {
            return Err(unsupported("raster pixel does not match bitmap format"));
        }
        let left = host_size(rect.x)?;
        let right = host_size(right)?;
        let top = host_size(rect.y)?;
        let rows = host_size(rect.height)?;
        let rgba_start = left.checked_mul(4)
            .ok_or_else(|| unsupported("raster fill offset overflow"))?;
        let rgba_end = right.checked_mul(4)
            .ok_or_else(|| unsupported("raster fill extent overflow"))?;
        let pixels = state.bitmaps[index].pixels.as_mut()
            .ok_or_else(|| unavailable("raster selected pixel backing disappeared"))?;
        // Layout and the entire rectangle were validated above; iteration uses
        // retained rows, so there is no fallible arithmetic after the first write.
        for row in pixels.chunks_exact_mut(info.stride).skip(top).take(rows) {
            match pixel {
                Pixel::Mono(white) => {
                    for x in left..right {
                        let mask = 0x80u8 >> (x % 8);
                        if white { row[x / 8] |= mask; } else { row[x / 8] &= !mask; }
                    }
                }
                Pixel::Rgba(rgba) => {
                    for target in row[rgba_start..rgba_end].chunks_exact_mut(4) {
                        target.copy_from_slice(&rgba);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn readback_rgba(&self, selection: &Selection) -> Result<RgbaImage, Stop> {
        let state = self.lock()?;
        let selection_index = state.selection(&self.shared, selection)?;
        let index = state.selected_index(selection_index)?
            .ok_or_else(|| unsupported("raster selection has no bitmap"))?;
        let record = &state.bitmaps[index];
        let info = record.info;
        let width = host_size(info.width)?;
        let bytes = width.checked_mul(host_size(info.height)?)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| unsupported("raster RGBA readback extent overflow"))?;
        if bytes > state.limits.max_bitmap_bytes {
            return Err(unsupported("raster RGBA readback exceeds the bitmap byte bound"));
        }
        let backing = record.pixels.as_ref()
            .ok_or_else(|| unavailable("raster selected pixel backing disappeared"))?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(bytes)
            .map_err(|_| unavailable("cannot reserve raster RGBA readback"))?;
        match info.format {
            PixelFormat::Rgba8Straight => pixels.extend_from_slice(backing),
            PixelFormat::Mono1Msb16 => {
                for row in backing.chunks_exact(info.stride) {
                    for x in 0..width {
                        let value = if row[x / 8] & (0x80u8 >> (x % 8)) == 0 { 0 } else { 255 };
                        pixels.extend_from_slice(&[value, value, value, 255]);
                    }
                }
            }
        }
        Ok(RgbaImage { width: info.width, height: info.height, pixels })
    }

    pub fn observe(&self, bitmap: &Bitmap) -> Result<BitmapObservation, Stop> {
        let state = self.lock()?;
        let index = state.bitmap(&self.shared, bitmap)?;
        Ok(state.observation(&self.shared, index))
    }

    pub fn observation_live(&self, observation: &BitmapObservation) -> Result<bool, Stop> {
        let state = self.lock()?;
        if !Weak::ptr_eq(&Arc::downgrade(&self.shared), &observation.shared) {
            return Err(unsupported("bitmap observation belongs to another raster device"));
        }
        // Internal inspection never returns/promotes an owning public handle.
        let Some(identity) = observation.identity.upgrade() else { return Ok(false); };
        Ok(state.bitmaps.get(identity.index).is_some_and(|record| {
            Arc::ptr_eq(&record.identity, &identity) && record.handle_live && record.pixels.is_some()
        }))
    }

    pub fn snapshot(&self) -> Result<Observation, Stop> {
        let state = self.lock()?;
        Ok(Observation {
            bitmap_issuances: state.bitmaps.len(),
            selection_issuances: state.selections.len(),
            live_bitmap_bytes: state.live_bytes()?,
        })
    }
}

#[cfg(test)]
mod tests;
