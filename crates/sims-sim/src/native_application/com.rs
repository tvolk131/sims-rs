//! Thread-bound Rust-port apartment authority for the recovered CoInitialize
//! call. This backend provides no Windows COM interoperability or class
//! activation, guest DLL/RPC/conversation cleanup or message pumping. Explicit
//! balance releases one genuine Rust STA obligation; last-owner Drop provides
//! fallback cleanup. Neither is a recovered native CoUninitialize call or
//! evidence of a constructor return, shutdown caller or application admission.
use super::unsupported;
use crate::saved_matrix::Stop;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    thread::ThreadId,
};

#[derive(Debug, Clone)]
pub struct Request {
    pub instruction: u32,
    pub import_slot: u32,
    pub argument: u32,
    pub return_pc: u32,
    pub application: u64,
    invocation: Rc<()>,
}
impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.instruction == other.instruction
            && self.import_slot == other.import_slot
            && self.argument == other.argument
            && self.return_pc == other.return_pc
            && self.application == other.application
            && Rc::ptr_eq(&self.invocation, &other.invocation)
    }
}
impl Eq for Request {}
impl Request {
    pub(super) fn new(application: u64) -> Self {
        Self {
            instruction: 0x4eed02,
            import_slot: 0x66641c,
            argument: 0,
            return_pc: 0x4eed08,
            application,
            invocation: Rc::new(()),
        }
    }
    pub fn validate(&self) -> Result<(), Stop> {
        if self.instruction != 0x4eed02
            || self.import_slot != 0x66641c
            || self.argument != 0
            || self.return_pc != 0x4eed08
            || self.application == 0
            || self
                .application
                .checked_add(324)
                .is_none_or(|end| end > 1_u64 << 32)
        {
            return Err(unsupported(
                "CoInitialize request differs from its recovered caller",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Apartment {
    thread: ThreadId,
    depth: u32,
    generation: u64,
}
thread_local! {
    static CURRENT: Rc<RefCell<Apartment>> = Rc::new(RefCell::new(Apartment {
        thread: std::thread::current().id(), depth: 0, generation: 0,
    }));
}

#[derive(Debug)]
struct Lease {
    apartment: Rc<RefCell<Apartment>>,
    generation: u64,
    // The Lease lives inside the shared Returned Rc. Every Return clone and
    // already-issued ApartmentAccess observes this same once-only obligation.
    active: Cell<bool>,
}
impl Lease {
    fn check_state(&self, state: &Apartment) -> Result<(), Stop> {
        if !self.active.get()
            || state.thread != std::thread::current().id()
            || state.depth == 0
            || state.generation != self.generation
            || !CURRENT.with(|current| Rc::ptr_eq(current, &self.apartment))
        {
            return Err(unsupported(
                "Rust-port apartment lease is not current on this thread",
            ));
        }
        Ok(())
    }
    fn check(&self) -> Result<(), Stop> {
        let state = self
            .apartment
            .try_borrow()
            .map_err(|_| Stop::Unavailable("Rust-port apartment is currently borrowed".into()))?;
        self.check_state(&state)
    }
    fn release(&self) -> Result<(), Stop> {
        let mut state = self
            .apartment
            .try_borrow_mut()
            .map_err(|_| Stop::Unavailable("Rust-port apartment is currently borrowed".into()))?;
        self.check_state(&state)?;
        // All checks precede both effects. Rc/Cell thread affinity prevents
        // another clone from interleaving; no fallible step follows release.
        state.depth -= 1;
        self.active.set(false);
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Released history can outlive this apartment generation. Do not even
        // borrow its state: a newer generation owns all remaining obligations.
        if !self.active.get() {
            return;
        }
        // Rc makes this obligation non-Send/non-Sync. No outside code can
        // borrow or change the private state during this last-owner cleanup.
        let mut state = self.apartment.borrow_mut();
        assert_eq!(state.thread, std::thread::current().id());
        assert_eq!(state.generation, self.generation);
        assert!(state.depth > 0);
        state.depth -= 1;
        self.active.set(false);
    }
}

#[derive(Debug)]
struct Returned {
    request: Request,
    hresult: u32,
    lease: Option<Lease>,
}

/// Immutable normal-return history with shared, revocable live authority.
/// Clones retain the same one obligation; they never reinitialize the apartment.
/// This is deliberately not serializable.
#[derive(Debug, Clone)]
pub struct Return(Rc<Returned>);
impl PartialEq for Return {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Return {}
impl Return {
    pub fn application(&self) -> u64 {
        self.0.request.application
    }
    pub fn hresult(&self) -> u32 {
        self.0.hresult
    }
    /// A host may acknowledge an actual failed call without an apartment
    /// lease. This declares a host result; it is not native execution evidence.
    pub fn failed(request: &Request, hresult: u32) -> Result<Self, Stop> {
        request.validate()?;
        if hresult & 0x8000_0000 == 0 {
            return Err(unsupported(
                "a successful CoInitialize return needs its live lease",
            ));
        }
        Ok(Self(Rc::new(Returned {
            request: request.clone(),
            hresult,
            lease: None,
        })))
    }
    pub fn validate(&self, request: &Request) -> Result<(), Stop> {
        request.validate()?;
        if self.0.request != *request
            || (self.0.hresult & 0x8000_0000 == 0) != self.0.lease.is_some()
        {
            return Err(unsupported(
                "CoInitialize return has another invocation or lease role",
            ));
        }
        // Historical replay must not compare an immutable return with the
        // thread's changing initialization count. Live access is separate.
        Ok(())
    }
    /// Explicitly balance this successful Rust-port initialization once,
    /// including S_FALSE. All retained clones immediately lose live access;
    /// immutable return validation remains valid. This performs no native COM
    /// teardown and admits no source shutdown caller or application execution.
    pub fn uninitialize_rust_port(&self) -> Result<(), Stop> {
        self.0.request.validate()?;
        let lease = self.0.lease.as_ref().ok_or_else(|| {
            Stop::Unavailable("failed CoInitialize return has no apartment obligation".into())
        })?;
        lease.release()
    }
    pub fn current_apartment(&self) -> Result<ApartmentAccess<'_>, Stop> {
        let lease = self.0.lease.as_ref().ok_or_else(|| {
            Stop::Unavailable("failed CoInitialize return has no apartment".into())
        })?;
        lease.check()?;
        Ok(ApartmentAccess { lease })
    }
}

/// A revocable reference to an actual retained Rust-port apartment. Call
/// `check` at use: an explicit release can revoke an already-issued access.
/// Possession grants no native COM binding or class-activation support.
pub struct ApartmentAccess<'a> {
    lease: &'a Lease,
}
impl ApartmentAccess<'_> {
    pub fn check(&self) -> Result<(), Stop> {
        self.lease.check()
    }
    /// Observation only; this does not check or confer live lease authority.
    pub fn thread(&self) -> ThreadId {
        self.lease.apartment.borrow().thread
    }
    /// Observation only; a released access may observe a later generation's
    /// count. Use `check` to authenticate this access before any operation.
    pub fn initialization_count(&self) -> u32 {
        self.lease.apartment.borrow().depth
    }
}

/// Acquire one STA initialization in the explicit Rust-port backend. Separate
/// service heaps on the same thread share this authority. Every invocation is
/// distinct; there is no address-based deduplication.
pub fn initialize_rust_port(request: &Request) -> Result<Return, Stop> {
    request.validate()?;
    CURRENT.with(|apartment| {
        let mut state = apartment
            .try_borrow_mut()
            .map_err(|_| Stop::Unavailable("Rust-port apartment is currently borrowed".into()))?;
        let next = state.depth.checked_add(1).ok_or_else(|| {
            Stop::Unavailable("Rust-port apartment initialization count exhausted".into())
        })?;
        let generation = if state.depth == 0 {
            state.generation.checked_add(1).ok_or_else(|| {
                Stop::Unavailable("Rust-port apartment generation exhausted".into())
            })?
        } else {
            state.generation
        };
        let hresult = u32::from(state.depth != 0);
        let lease = Lease {
            apartment: Rc::clone(apartment),
            generation,
            active: Cell::new(true),
        };
        // No Result::Err follows this effect. One immutable return owns the
        // obligation, including while it is staged or retained for replay.
        state.depth = next;
        state.generation = generation;
        drop(state);
        Ok(Return(Rc::new(Returned {
            request: request.clone(),
            hresult,
            lease: Some(lease),
        })))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Authored UNRUN by the isolated worker. Parent owns all execution.
    fn state_snapshot() -> (ThreadId, u32, u64) {
        CURRENT.with(|apartment| {
            let state = apartment.borrow();
            (state.thread, state.depth, state.generation)
        })
    }

    #[test]
    fn clones_share_one_obligation_and_separate_calls_share_the_current_apartment() {
        let request = Request::new(0x10000000);
        let first = initialize_rust_port(&request).unwrap();
        assert_eq!(first.hresult(), 0);
        let clone = first.clone();
        let second = initialize_rust_port(&request).unwrap();
        assert_eq!(second.hresult(), 1);
        assert_eq!(first.current_apartment().unwrap().initialization_count(), 2);
        drop(first);
        assert_eq!(clone.current_apartment().unwrap().initialization_count(), 2);
        drop(clone);
        assert_eq!(
            second.current_apartment().unwrap().initialization_count(),
            1
        );
        // Immutable return validation is independent of the current depth.
        second.validate(&request).unwrap();
        drop(second);
        assert_eq!(initialize_rust_port(&request).unwrap().hresult(), 0);
    }

    #[test]
    fn another_rust_thread_has_its_own_apartment_and_failures_acquire_no_lease() {
        let request = Request::new(0x10000000);
        let active = initialize_rust_port(&request).unwrap();
        let thread = std::thread::spawn(|| {
            let own = initialize_rust_port(&Request::new(0x10000000)).unwrap();
            (
                own.hresult(),
                own.current_apartment().unwrap().initialization_count(),
            )
        });
        assert_eq!(thread.join().unwrap(), (0, 1));
        let failed = Return::failed(&request, 0x80010106).unwrap();
        failed.validate(&request).unwrap();
        assert!(failed.current_apartment().is_err());
        assert!(Return::failed(&request, 0).is_err());
        assert_eq!(
            active.current_apartment().unwrap().initialization_count(),
            1
        );
    }

    #[test]
    fn explicit_release_revokes_every_clone_and_previously_issued_access() {
        let request = Request::new(0x10000000);
        let returned = initialize_rust_port(&request).unwrap();
        let clone = returned.clone();
        let access = returned.current_apartment().unwrap();
        let clone_access = clone.current_apartment().unwrap();
        assert_eq!(access.initialization_count(), 1);
        let (): () = clone.uninitialize_rust_port().unwrap();
        assert_eq!(state_snapshot().1, 0);
        assert!(returned.current_apartment().is_err());
        assert!(clone.current_apartment().is_err());
        assert!(access.check().is_err());
        assert!(clone_access.check().is_err());
        // Observations remain compatible, but grant no execution authority.
        assert_eq!(access.thread(), std::thread::current().id());
        assert_eq!(access.initialization_count(), 0);
        returned.validate(&request).unwrap();
        clone.validate(&request).unwrap();
        assert_eq!((returned.application(), returned.hresult()), (request.application, 0));
        let before = state_snapshot();
        assert!(returned.uninitialize_rust_port().is_err());
        assert!(clone.uninitialize_rust_port().is_err());
        assert_eq!(state_snapshot(), before);
    }

    #[test]
    fn explicit_release_balances_s_ok_and_nested_s_false_independently_in_either_order() {
        for release_first in [true, false] {
            let request = Request::new(0x10000000);
            let first = initialize_rust_port(&request).unwrap();
            let nested = initialize_rust_port(&request).unwrap();
            assert_eq!((first.hresult(), nested.hresult()), (0, 1));
            let (released, survivor) = if release_first {
                (&first, &nested)
            } else {
                (&nested, &first)
            };
            released.uninitialize_rust_port().unwrap();
            assert!(released.current_apartment().is_err());
            assert_eq!(survivor.current_apartment().unwrap().initialization_count(), 1);
            let failed = Return::failed(&request, 0x80010106).unwrap();
            let before = state_snapshot();
            assert!(failed.uninitialize_rust_port().is_err());
            assert!(released.uninitialize_rust_port().is_err());
            assert_eq!(state_snapshot(), before);
            failed.validate(&request).unwrap();
            first.validate(&request).unwrap();
            nested.validate(&request).unwrap();
            survivor.uninitialize_rust_port().unwrap();
            assert_eq!(state_snapshot().1, 0);
        }
    }

    #[test]
    fn explicit_release_refuses_shared_and_exclusive_state_borrows_before_effects() {
        let request = Request::new(0x10000000);
        let returned = initialize_rust_port(&request).unwrap();
        let lease = returned.0.lease.as_ref().unwrap();
        let apartment = Rc::clone(&lease.apartment);
        let before = state_snapshot();
        let shared = apartment.borrow();
        assert!(matches!(returned.uninitialize_rust_port(), Err(Stop::Unavailable(_))));
        assert!(lease.active.get());
        assert_eq!((shared.thread, shared.depth, shared.generation), before);
        returned.current_apartment().unwrap().check().unwrap();
        drop(shared);
        let exclusive = apartment.borrow_mut();
        assert!(matches!(returned.uninitialize_rust_port(), Err(Stop::Unavailable(_))));
        assert!(returned.current_apartment().is_err());
        assert!(lease.active.get());
        assert_eq!((exclusive.thread, exclusive.depth, exclusive.generation), before);
        drop(exclusive);
        assert_eq!(state_snapshot(), before);
        returned.uninitialize_rust_port().unwrap();
        assert_eq!(state_snapshot().1, 0);
    }

    #[test]
    fn explicit_release_checks_thread_generation_and_nonzero_depth_before_effects() {
        let request = Request::new(0x10000000);
        let returned = initialize_rust_port(&request).unwrap();
        let access = returned.current_apartment().unwrap();
        let lease = returned.0.lease.as_ref().unwrap();
        let before = state_snapshot();
        // Only private test state is perturbed; the Return was genuinely issued.
        let other_thread = std::thread::spawn(|| std::thread::current().id()).join().unwrap();
        CURRENT.with(|apartment| apartment.borrow_mut().thread = other_thread);
        let wrong_thread = returned.uninitialize_rust_port();
        let access_wrong_thread = access.check();
        let after_thread = state_snapshot();
        CURRENT.with(|apartment| apartment.borrow_mut().thread = before.0);
        assert!(wrong_thread.is_err());
        assert!(access_wrong_thread.is_err());
        assert_eq!(after_thread, (other_thread, before.1, before.2));
        assert!(lease.active.get());

        CURRENT.with(|apartment| apartment.borrow_mut().generation = before.2 + 1);
        let wrong_generation = returned.uninitialize_rust_port();
        let access_wrong_generation = access.check();
        let after_generation = state_snapshot();
        CURRENT.with(|apartment| apartment.borrow_mut().generation = before.2);
        assert!(wrong_generation.is_err());
        assert!(access_wrong_generation.is_err());
        assert_eq!(after_generation, (before.0, before.1, before.2 + 1));
        assert!(lease.active.get());

        CURRENT.with(|apartment| apartment.borrow_mut().depth = 0);
        let zero_depth = returned.uninitialize_rust_port();
        let access_zero_depth = access.check();
        let after_depth = state_snapshot();
        CURRENT.with(|apartment| apartment.borrow_mut().depth = before.1);
        assert!(zero_depth.is_err());
        assert!(access_zero_depth.is_err());
        assert_eq!(after_depth, (before.0, 0, before.2));
        assert!(lease.active.get());
        access.check().unwrap();
        returned.uninitialize_rust_port().unwrap();
    }

    #[test]
    fn released_history_cannot_affect_a_later_generation_even_while_borrowed() {
        let request = Request::new(0x10000000);
        let old = initialize_rust_port(&request).unwrap();
        let retained = old.clone();
        let access = old.current_apartment().unwrap();
        let generation = state_snapshot().2;
        old.uninitialize_rust_port().unwrap();
        let later = initialize_rust_port(&request).unwrap();
        assert_eq!(later.hresult(), 0);
        assert_eq!(state_snapshot().2, generation + 1);
        let before = state_snapshot();
        assert!(access.check().is_err());
        assert!(old.current_apartment().is_err());
        assert!(retained.uninitialize_rust_port().is_err());
        assert_eq!(state_snapshot(), before);
        old.validate(&request).unwrap();
        retained.validate(&request).unwrap();
        assert!(old.validate(&Request::new(request.application)).is_err());
        drop(access);
        // The final released owner must skip borrowing entirely, and cannot
        // assert on the newer generation or decrement its live initialization.
        let apartment = CURRENT.with(Rc::clone);
        let borrowed = apartment.borrow_mut();
        drop(old);
        drop(retained);
        assert_eq!((borrowed.thread, borrowed.depth, borrowed.generation), before);
        drop(borrowed);
        assert_eq!(later.current_apartment().unwrap().initialization_count(), 1);
        drop(later);
        assert_eq!(state_snapshot().1, 0);
    }

    #[test]
    fn initialization_overflow_generation_and_profile_refusals_preserve_existing_obligations() {
        let request = Request::new(0x10000000);
        let returned = initialize_rust_port(&request).unwrap();
        let before = state_snapshot();
        CURRENT.with(|apartment| apartment.borrow_mut().depth = u32::MAX);
        let overflow = initialize_rust_port(&request);
        let after_overflow = state_snapshot();
        CURRENT.with(|apartment| apartment.borrow_mut().depth = before.1);
        assert!(matches!(overflow, Err(Stop::Unavailable(_))));
        assert_eq!(after_overflow, (before.0, u32::MAX, before.2));
        let mut invalid = request.clone();
        invalid.argument = 1;
        assert!(initialize_rust_port(&invalid).is_err());
        assert_eq!(state_snapshot(), before);
        returned.current_apartment().unwrap().check().unwrap();
        returned.uninitialize_rust_port().unwrap();

        let empty = state_snapshot();
        CURRENT.with(|apartment| apartment.borrow_mut().generation = u64::MAX);
        let exhausted = initialize_rust_port(&request);
        let after_exhaustion = state_snapshot();
        CURRENT.with(|apartment| apartment.borrow_mut().generation = empty.2);
        assert!(matches!(exhausted, Err(Stop::Unavailable(_))));
        assert_eq!(after_exhaustion, (empty.0, 0, u64::MAX));
        assert_eq!(state_snapshot(), empty);
        let later = initialize_rust_port(&request).unwrap();
        assert_eq!(later.hresult(), 0);
        drop(returned);
        assert_eq!(later.current_apartment().unwrap().initialization_count(), 1);
        // Unreleased calls still use the original last-owner Drop fallback.
        drop(later);
        assert_eq!(state_snapshot().1, 0);
    }
}
