//! Exact managed-window observation, never execution or kernel authority.
//!
//! Parent must construct one reducer from its actual root-window lifecycle,
//! forward generation/close changes and Tick expiry, and route the sealed task
//! reply back to that same reducer. There is no discovery or root reselection.
//! Acceptance yields only an UNRECORDED candidate for immediate parent emission.
//! A missing/dropped/rejected journal write leaves it externally unavailable;
//! only the parent can establish publication through its bounded journal ack.
//! This leaf neither publishes nor certifies exact-ID/GUI acceptance.

use iced::{Task, window};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unavailable {
    Platform,
    NotMainThread,
    WindowHandle,
    HandlePlatform,
    Selector,
    DetachedView,
    NonPositiveNumber,
    NumberOutOfRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QueryError {
    Busy,
    Closed,
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rejection {
    Closed,
    Exhausted,
    ForeignLifetime,
    ForeignWindow,
    StaleGeneration,
    NoPendingQuery,
    StaleQuery,
    Expired,
    InvalidTime,
    Unavailable(Unavailable),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lifecycle {
    Open,
    Closed,
    Exhausted,
}

// An allocation identity, not a native pointer, global counter or serialized ID.
// No constructor/fields for this seal or Ticket are visible to the parent.
#[derive(Debug)]
struct Lifetime;

#[derive(Clone, Debug)]
struct Ticket {
    lifetime: Arc<Lifetime>,
    root: window::Id,
    generation: u64,
    serial: u64,
}

impl Ticket {
    fn complete(self, result: Result<u32, Unavailable>, observed_at: Instant) -> Reply {
        Reply { ticket: self, result, observed_at }
    }
}

/// Opaque and cloneable for Iced parent messages. A clone is still one-shot at
/// acceptance: the first matching reply retires the pending query. Never serde.
#[derive(Clone, Debug)]
pub(super) struct Reply {
    ticket: Ticket,
    result: Result<u32, Unavailable>,
    observed_at: Instant,
}

#[derive(Debug)]
struct Pending {
    serial: u64,
    issued_at: Instant,
    deadline: Instant,
}

/// Deliberately not a recorded/verified/available type, and not serializable.
/// Parent may emit only the scalar observation; root/generation guard routing.
/// Do not retain it across lifecycle changes or treat it as publication proof.
#[derive(Debug)]
#[must_use = "UNRECORDED: missing or rejected parent journal publication is unavailable"]
pub(super) struct UnrecordedCandidate {
    root: window::Id,
    generation: u64,
    window_number: u32,
}

impl UnrecordedCandidate {
    pub fn root(&self) -> window::Id { self.root }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn window_number(&self) -> u32 { self.window_number }
    pub fn publication_status(&self) -> &'static str { "UNRECORDED" }
}

/// One instance per actual managed root lifetime. Not Clone; no reset, adopt,
/// rebind, raw-number ingestion, cached-candidate getter or publication setter.
#[derive(Debug)]
pub(super) struct WindowIdentity {
    root: window::Id,
    lifetime: Arc<Lifetime>,
    generation: u64,
    next_serial: u64,
    lifecycle: Lifecycle,
    pending: Option<Pending>,
}

impl WindowIdentity {
    /// Call exactly once when parent learns its actual managed root Id. Parent
    /// must not reconstruct this state to revive that root after close/failure.
    pub fn new(root: window::Id, generation: u64) -> Self {
        Self {
            root,
            lifetime: Arc::new(Lifetime),
            generation,
            next_serial: 1,
            lifecycle: Lifecycle::Open,
            pending: None,
        }
    }

    pub fn root(&self) -> window::Id { self.root }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn has_pending_query(&self) -> bool { self.pending.is_some() }

    fn require_open(&self) -> Result<(), QueryError> {
        match self.lifecycle {
            Lifecycle::Open => Ok(()),
            Lifecycle::Closed => Err(QueryError::Closed),
            Lifecycle::Exhausted => Err(QueryError::Exhausted),
        }
    }

    fn exhaust(&mut self) -> QueryError {
        self.pending = None;
        self.lifecycle = Lifecycle::Exhausted;
        QueryError::Exhausted
    }

    /// Generation is supplied, not incremented here. Any change invalidates
    /// pending work, including a reused numeric generation. Query serials never
    /// reset, so cycling that number cannot revive a previous ticket.
    pub fn set_generation(&mut self, generation: u64) -> Result<bool, QueryError> {
        self.require_open()?;
        if generation == self.generation { return Ok(false); }
        self.generation = generation;
        self.pending = None;
        Ok(true)
    }

    /// Parent calls at close intent, before dispatching its actual close task.
    pub fn close_intent(&mut self, id: window::Id) -> bool {
        if id != self.root { return false; }
        self.pending = None;
        self.lifecycle = Lifecycle::Closed;
        true
    }

    /// Also forward the exact managed window's Closed event, including closes
    /// with no prior intent. No reply is required for either close transition.
    pub fn closed(&mut self, id: window::Id) -> bool {
        self.close_intent(id)
    }

    /// Nonblocking Tick maintenance. Expiry does not schedule a retry. Use a
    /// fresh Instant from the parent's monotonic clock, as for request/accept.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.pending.as_ref().is_some_and(|pending| now >= pending.deadline) {
            self.pending = None;
            true
        } else {
            false
        }
    }

    fn issue(&mut self, now: Instant) -> Result<Ticket, QueryError> {
        self.require_open()?;
        self.expire(now);
        if self.pending.is_some() { return Err(QueryError::Busy); }
        let Some(next) = self.next_serial.checked_add(1) else {
            return Err(self.exhaust());
        };
        let Some(deadline) = now.checked_add(QUERY_TIMEOUT) else {
            return Err(self.exhaust());
        };
        let serial = self.next_serial;
        self.next_serial = next;
        self.pending = Some(Pending { serial, issued_at: now, deadline });
        Ok(Ticket {
            lifetime: Arc::clone(&self.lifetime),
            root: self.root,
            generation: self.generation,
            serial,
        })
    }

    /// Uses Iced 0.14's exact managed-window callback. If it never runs, close
    /// or Tick expiry retires pending work; no wait or completion assumption.
    pub fn request(&mut self, now: Instant) -> Result<Task<Reply>, QueryError> {
        let ticket = self.issue(now)?;
        Ok(window::run(self.root, move |managed_window| {
            // All borrowed/native handles stay inside this synchronous call.
            let result = native_window_number(managed_window);
            ticket.complete(result, Instant::now())
        }))
    }

    /// Consumes a genuine issued callback reply, returning an UNRECORDED scalar
    /// candidate only. The reducer keeps no candidate to expose after a lost
    /// publication. Parent must synchronously attempt its bounded journal write
    /// and require an ack before external availability; absent ack means absent.
    pub fn accept(
        &mut self,
        reply: Reply,
        now: Instant,
    ) -> Result<UnrecordedCandidate, Rejection> {
        match self.lifecycle {
            Lifecycle::Closed => return Err(Rejection::Closed),
            Lifecycle::Exhausted => return Err(Rejection::Exhausted),
            Lifecycle::Open => {}
        }
        if !Arc::ptr_eq(&self.lifetime, &reply.ticket.lifetime) {
            return Err(Rejection::ForeignLifetime);
        }
        if reply.ticket.root != self.root { return Err(Rejection::ForeignWindow); }
        if reply.ticket.generation != self.generation {
            return Err(Rejection::StaleGeneration);
        }
        let Some(pending) = self.pending.as_ref() else {
            return Err(Rejection::NoPendingQuery);
        };
        if reply.ticket.serial != pending.serial { return Err(Rejection::StaleQuery); }
        // Only a matching reply may consume this pending query. Stale/foreign
        // messages must not evict a newer legitimate query.
        let Pending { issued_at, deadline, .. } = self.pending.take().unwrap();
        if now >= deadline || reply.observed_at >= deadline {
            return Err(Rejection::Expired);
        }
        if now < issued_at || reply.observed_at < issued_at || reply.observed_at > now {
            return Err(Rejection::InvalidTime);
        }
        let window_number = reply.result.map_err(Rejection::Unavailable)?;
        // Defense in depth; the only production producer already checks this.
        if window_number == 0 {
            return Err(Rejection::Unavailable(Unavailable::NonPositiveNumber));
        }
        Ok(UnrecordedCandidate {
            root: self.root,
            generation: self.generation,
            window_number,
        })
    }
}

fn checked_window_number(number: isize) -> Result<u32, Unavailable> {
    if number <= 0 { return Err(Unavailable::NonPositiveNumber); }
    u32::try_from(number).map_err(|_| Unavailable::NumberOutOfRange)
}

#[cfg(not(target_os = "macos"))]
fn native_window_number(_: &dyn window::Window) -> Result<u32, Unavailable> {
    // In particular, do not even ask for a raw handle on another platform.
    Err(Unavailable::Platform)
}

#[cfg(target_os = "macos")]
fn native_window_number(managed_window: &dyn window::Window) -> Result<u32, Unavailable> {
    use window::raw_window_handle::RawWindowHandle;

    // Check actual thread identity before obtaining/dereferencing any AppKit
    // handle. Do not rely solely on Iced's normal event-loop thread behavior.
    if !appkit::is_main_thread() { return Err(Unavailable::NotMainThread); }
    let handle = managed_window.window_handle().map_err(|_| Unavailable::WindowHandle)?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(Unavailable::HandlePlatform);
    };
    // SAFETY: the borrowed managed WindowHandle guarantees a live NSView;
    // actual main-thread identity was checked above. The pointer is neither
    // retained nor stored nor returned; both public getters finish here.
    let number = unsafe { appkit::number_from_view(handle.ns_view.as_ptr()) }?;
    checked_window_number(number)
}

#[cfg(target_os = "macos")]
mod appkit {
    use super::Unavailable;
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    struct Selector { _opaque: [u8; 0] }
    #[repr(C)]
    struct NSView { _opaque: [u8; 0] }
    #[repr(C)]
    struct NSWindow { _opaque: [u8; 0] }

    #[link(name = "System")]
    unsafe extern "C" {
        fn pthread_main_np() -> c_int;
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn sel_registerName(name: *const c_char) -> *const Selector;
        fn objc_msgSend();
    }

    pub(super) fn is_main_thread() -> bool {
        // SAFETY: pthread_main_np has no preconditions or pointer arguments.
        unsafe { pthread_main_np() != 0 }
    }

    /// Only call with a borrowed live NSView on the actual AppKit main thread.
    /// All native pointers remain local to this synchronous helper/callback.
    pub(super) unsafe fn number_from_view(view: *mut c_void) -> Result<isize, Unavailable> {
        // NSView.h: -window -> nullable NSWindow*, no extra arguments.
        type GetWindow = unsafe extern "C" fn(*mut NSView, *const Selector) -> *mut NSWindow;
        // NSWindow.h: -windowNumber -> NSInteger (pointer-width signed integer
        // on supported macOS ABIs), no extra arguments. No stret/fpret ABI.
        type GetNumber = unsafe extern "C" fn(*mut NSWindow, *const Selector) -> isize;

        // SAFETY: literal NUL-terminated selector names, and objc_msgSend is
        // cast separately to each selector's exact receiver/return ABI. These
        // are public scalar/object getters on the supplied borrowed objects.
        // No Rust callback is exported, and all FFI uses non-unwinding C ABI.
        unsafe {
            let window_selector = sel_registerName(c"window".as_ptr());
            let number_selector = sel_registerName(c"windowNumber".as_ptr());
            if window_selector.is_null() || number_selector.is_null() {
                return Err(Unavailable::Selector);
            }
            let get_window: GetWindow = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let get_number: GetNumber = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let window = get_window(view.cast::<NSView>(), window_selector);
            if window.is_null() { return Err(Unavailable::DetachedView); }
            Ok(get_number(window, number_selector))
        }
    }
}

#[cfg(test)]
#[path = "window_identity/tests.rs"]
mod tests;

// Complete only the exact genuine pending ticket created by request(). This
// test-only fixture never issues/adopts a ticket, changes the pending query or
// invokes the Run closure/native getter. Production privacy is unchanged.
#[cfg(test)]
impl WindowIdentity {
    pub(super) fn pending_reply_for_test(
        &self,
        result: Result<u32, Unavailable>,
        observed_at: Instant,
    ) -> Option<Reply> {
        let pending = self.pending.as_ref()?;
        Some(Ticket {
            lifetime: Arc::clone(&self.lifetime),
            root: self.root,
            generation: self.generation,
            serial: pending.serial,
        }.complete(result, observed_at))
    }
}
