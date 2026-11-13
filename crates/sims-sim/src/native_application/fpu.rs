//! Explicitly initialized, thread-bound Rust-port control-instruction context.
//!
//! These bounded FWAIT/FNSTCW/FLDCW operations neither access host FPU registers
//! nor emulate arithmetic or deliver native exceptions. The initial control
//! word and exception state are declared input, never an ambient/default word.
//! Source continuation remains the containing cursor's integration obligation.
use super::{Owner, com, disjoint, mutex, span, unsupported};
use crate::saved_matrix::Stop;
use std::{
    cell::{Ref, RefCell, RefMut},
    rc::{Rc, Weak},
    thread::ThreadId,
};

// A bounded port profile, not a native instruction limit. Reads and loads share
// this budget; failed attempts consume neither a serial nor an operation.
const MAX_OPERATIONS: u64 = 4096;

#[derive(Debug, Clone)]
pub(super) struct Invocation {
    application: u64,
    identity: Rc<()>,
}
impl Invocation {
    pub(super) fn new(application: u64) -> Self {
        Self {
            application,
            identity: Rc::new(()),
        }
    }
}
impl PartialEq for Invocation {
    fn eq(&self, other: &Self) -> bool {
        self.application == other.application && Rc::ptr_eq(&self.identity, &other.identity)
    }
}
impl Eq for Invocation {}

/// Sealed FWAIT4eedc6/FNSTCW4eedc7 source entry. Only the containing source
/// cursor can supply its invocation and complete post-mutex owner images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRequest {
    invocation: Invocation,
    application: Owner,
    com: com::Return,
    mutex: Option<mutex::Return>,
    wrapper: Option<Owner>,
}
impl ReadRequest {
    pub(super) fn new(
        invocation: &Invocation,
        application: &Owner,
        com: &com::Return,
        mutex: Option<&mutex::Return>,
        wrapper: Option<&Owner>,
    ) -> Self {
        Self {
            invocation: invocation.clone(),
            application: application.clone(),
            com: com.clone(),
            mutex: mutex.cloned(),
            wrapper: wrapper.cloned(),
        }
    }
    pub fn validate(&self) -> Result<(), Stop> {
        span(&self.application)?;
        if self.application.data.len() != 324
            || self.invocation.application != self.application.token
            || self.com.application() != self.application.token
            || self.application.data[248] != 0
        {
            return Err(unsupported(
                "FPU read lacks its complete post-mutex application and COM identity",
            ));
        }
        match (&self.mutex, &self.wrapper) {
            // This sealed pair represents the cursor's NULL allocation path;
            // an API-null CreateMutex return still requires its wrapper image.
            (None, None) => Ok(()),
            (Some(returned), Some(wrapper)) => {
                returned.validate(returned.request())?;
                span(wrapper)?;
                disjoint(vec![span(&self.application)?, span(wrapper)?])?;
                let source = returned.request();
                let mut application = source.application_image().clone();
                application.data[248] = 0; // 4eedb8, after mutex publication
                let mut completed = source.wrapper_image().clone();
                completed.data[4..8].copy_from_slice(&returned.word().to_le_bytes());
                if returned.word() != 0 {
                    completed.data[8..12].copy_from_slice(&1_u32.to_le_bytes());
                }
                if self.application != application
                    || self.com != *source.com_return()
                    || *wrapper != completed
                {
                    return Err(unsupported(
                        "FPU read differs from its completed mutex source/COM/wrapper history",
                    ));
                }
                Ok(())
            }
            _ => Err(unsupported(
                "FPU read must retain both the mutex return and its completed wrapper",
            )),
        }
    }
    pub fn application_image(&self) -> &Owner {
        &self.application
    }
    pub fn com_return(&self) -> &com::Return {
        &self.com
    }
    pub fn mutex_return(&self) -> Option<&mutex::Return> {
        self.mutex.as_ref()
    }
    pub fn wrapper_image(&self) -> Option<&Owner> {
        self.wrapper.as_ref()
    }
}

/// Pending exception flags are explicit six-bit input. Unknown state prevents
/// waiting operations; no flags or native exception delivery are invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionState {
    Unknown,
    Flags(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialState {
    pub control_word: u16,
    pub exceptions: ExceptionState,
}

fn wait(control: u16, exceptions: ExceptionState) -> Result<(), Stop> {
    match exceptions {
        ExceptionState::Unknown => Err(Stop::Unavailable(
            "Rust-port FWAIT exception state is unknown".into(),
        )),
        ExceptionState::Flags(flags) if flags & !0x3f != 0 => Err(unsupported(
            "Rust-port exception state exceeds the six pending flags",
        )),
        ExceptionState::Flags(flags) if u16::from(flags) & !control & 0x3f != 0 => Err(
            Stop::Unavailable("Rust-port FWAIT has a known pending unmasked exception".into()),
        ),
        ExceptionState::Flags(_) => Ok(()),
    }
}

#[derive(Debug)]
struct Identity {
    thread: ThreadId,
}
#[derive(Debug)]
struct State {
    live: bool,
    control_word: u16,
    exceptions: ExceptionState,
    serial: u64,
}
impl State {
    fn next_serial(&self) -> Result<u64, Stop> {
        let next = self.serial.checked_add(1).ok_or_else(|| {
            Stop::Unavailable("Rust-port control operation serial exhausted".into())
        })?;
        if next > MAX_OPERATIONS {
            return Err(Stop::Unavailable(
                "Rust-port control operation quota exhausted".into(),
            ));
        }
        Ok(next)
    }
}
#[derive(Debug)]
struct Context {
    identity: Rc<Identity>,
    state: RefCell<State>,
}
thread_local! {
    // No context/state exists until explicit installation. The thread slot and
    // histories are weak owners; only actual Port handles keep a context live.
    static CURRENT: RefCell<Weak<Context>> = RefCell::new(Weak::new());
}
impl Context {
    fn check_current(&self) -> Result<(), Stop> {
        if self.identity.thread != std::thread::current().id() {
            return Err(unsupported(
                "Rust-port control context belongs to another thread",
            ));
        }
        let current = CURRENT.with(|slot| {
            slot.try_borrow()
                .map(|current| std::ptr::eq(current.as_ptr(), self))
                .map_err(|_| Stop::Unavailable("Rust-port context slot is borrowed".into()))
        })?;
        if !current {
            return Err(Stop::Unavailable(
                "Rust-port control context is no longer installed".into(),
            ));
        }
        Ok(())
    }
    fn borrow_live(&self) -> Result<Ref<'_, State>, Stop> {
        self.check_current()?;
        let state = self.state.try_borrow().map_err(|_| {
            Stop::Unavailable("Rust-port control context is currently borrowed".into())
        })?;
        if !state.live {
            return Err(Stop::Unavailable(
                "Rust-port control context is retired".into(),
            ));
        }
        Ok(state)
    }
    fn borrow_live_mut(&self) -> Result<RefMut<'_, State>, Stop> {
        self.check_current()?;
        let state = self.state.try_borrow_mut().map_err(|_| {
            Stop::Unavailable("Rust-port control context is currently borrowed".into())
        })?;
        if !state.live {
            return Err(Stop::Unavailable(
                "Rust-port control context is retired".into(),
            ));
        }
        Ok(state)
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        // Last actual Port owner: retire without restoring any previous word.
        // Receipt identities survive independently and cannot retain this state.
        self.state.get_mut().live = false;
    }
}

/// Cloneable handles to one actual thread-bound control context. Installing a
/// second live context fails; cloning never installs or resets state.
///
/// ```compile_fail
/// use sims_sim::native_application::fpu::Port;
/// fn require_send<T: Send>() {}
/// require_send::<Port>();
/// ```
/// ```compile_fail
/// use sims_sim::native_application::fpu::Port;
/// fn require_sync<T: Sync>() {}
/// require_sync::<Port>();
/// ```
#[derive(Debug, Clone)]
pub struct Port(Rc<Context>);
impl Port {
    pub fn install(initial: InitialState) -> Result<Self, Stop> {
        if matches!(initial.exceptions, ExceptionState::Flags(flags) if flags & !0x3f != 0) {
            return Err(unsupported(
                "Rust-port exception state exceeds the six pending flags",
            ));
        }
        CURRENT.with(|slot| {
            let mut current = slot.try_borrow_mut().map_err(|_| {
                Stop::Unavailable("Rust-port context slot is currently borrowed".into())
            })?;
            if let Some(context) = current.upgrade() {
                let state = context.state.try_borrow().map_err(|_| {
                    Stop::Unavailable("Rust-port control context is currently borrowed".into())
                })?;
                if state.live {
                    return Err(Stop::Unavailable(
                        "this thread already has a live Rust-port control context".into(),
                    ));
                }
            }
            let context = Rc::new(Context {
                identity: Rc::new(Identity {
                    thread: std::thread::current().id(),
                }),
                state: RefCell::new(State {
                    live: true,
                    control_word: initial.control_word,
                    exceptions: initial.exceptions,
                    serial: 0,
                }),
            });
            *current = Rc::downgrade(&context);
            Ok(Self(context))
        })
    }
    pub fn check_live(&self) -> Result<(), Stop> {
        self.0.borrow_live().map(|_| ())
    }
    /// Inspect declared current port state; this is not an FNSTCW receipt.
    pub fn control_word(&self) -> Result<u16, Stop> {
        Ok(self.0.borrow_live()?.control_word)
    }
    /// Historical issuing identity, including after retirement. Live access
    /// additionally requires check_live/current_word or an actual operation.
    pub fn owns_read(&self, returned: &ReadReturn) -> bool {
        returned.0.history.belongs_to(&self.0) && returned.validate(returned.request()).is_ok()
    }
    pub fn read_control(&self, request: &ReadRequest) -> Result<ReadReturn, Stop> {
        request.validate()?;
        let mut state = self.0.borrow_live_mut()?;
        wait(state.control_word, state.exceptions)?;
        let serial = state.next_serial()?;
        let returned = ReadReturn(Rc::new(ReadOperation {
            request: request.clone(),
            raw: state.control_word,
            serial,
            exceptions: state.exceptions,
            history: History::new(&self.0),
        }));
        // Receipt allocated before commit. No Result::Err follows this effect.
        state.serial = serial;
        Ok(returned)
    }
    pub fn load_control(&self, request: &LoadRequest) -> Result<LoadReturn, Stop> {
        request.validate()?;
        if !request.read.0.history.belongs_to(&self.0) {
            return Err(unsupported(
                "FLDCW request belongs to another control context",
            ));
        }
        let mut state = self.0.borrow_live_mut()?;
        // Check the PREEXISTING masks/flags, even when the requested load would
        // mask every exception. Loading must not erase a pending wait failure.
        wait(state.control_word, state.exceptions)?;
        let serial = state.next_serial()?;
        let returned = LoadReturn(Rc::new(LoadOperation {
            request: request.clone(),
            before: state.control_word,
            after: request.control,
            serial,
            exceptions: state.exceptions,
        }));
        state.control_word = request.control;
        state.serial = serial;
        // Exception flags are preserved. No fallible work after the commit.
        Ok(returned)
    }
    pub fn retire(&self) -> Result<(), Stop> {
        self.0.borrow_live_mut()?.live = false;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct History {
    identity: Rc<Identity>,
    context: Weak<Context>,
}
impl History {
    fn new(context: &Rc<Context>) -> Self {
        Self {
            identity: Rc::clone(&context.identity),
            context: Rc::downgrade(context),
        }
    }
    fn belongs_to(&self, context: &Rc<Context>) -> bool {
        Rc::ptr_eq(&self.identity, &context.identity)
            && Weak::ptr_eq(&self.context, &Rc::downgrade(context))
    }
    fn current_word(&self) -> Result<u16, Stop> {
        let context = self.context.upgrade().ok_or_else(|| {
            Stop::Unavailable("Rust-port control context has no remaining port owner".into())
        })?;
        if !self.belongs_to(&context) {
            return Err(unsupported("control history has another context identity"));
        }
        let word = context.borrow_live()?.control_word;
        Ok(word)
    }
}

#[derive(Debug)]
struct ReadOperation {
    request: ReadRequest,
    raw: u16,
    serial: u64,
    exceptions: ExceptionState,
    history: History,
}
/// An immutable successful wait/store operation. Only Port::read_control can
/// issue one; equal raw words do not make independent operations equal.
#[derive(Debug, Clone)]
pub struct ReadReturn(Rc<ReadOperation>);
impl PartialEq for ReadReturn {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ReadReturn {}
impl ReadReturn {
    pub fn request(&self) -> &ReadRequest {
        &self.0.request
    }
    pub fn raw(&self) -> u16 {
        self.0.raw
    }
    pub fn serial(&self) -> u64 {
        self.0.serial
    }
    pub fn validate(&self, request: &ReadRequest) -> Result<(), Stop> {
        request.validate()?;
        if self.0.request != *request || self.0.serial == 0 || self.0.serial > MAX_OPERATIONS {
            return Err(unsupported(
                "FNSTCW return has another invocation or operation serial",
            ));
        }
        // Immutable wait evidence, never a check against today's context.
        wait(self.0.raw, self.0.exceptions)
    }
    pub fn current_word(&self) -> Result<u16, Stop> {
        self.0.history.current_word()
    }
}

pub fn changed(raw: u16) -> bool {
    raw & 0x0300 != 0 || raw & 0x003f != 0x003f || raw & 0x0c00 != 0
}
pub fn normalized(raw: u16) -> u16 {
    ((raw & 0xfcff) | 0x003f) & 0xf3ff
}

/// Sealed changed-branch FLDCW4eedf5 entry; its operand derives only from an
/// actual read's original word, even if later operations change live state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    read: ReadReturn,
    control: u16,
}
impl LoadRequest {
    pub(super) fn new(read: &ReadReturn) -> Self {
        Self {
            read: read.clone(),
            control: normalized(read.raw()),
        }
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.read.validate(self.read.request())?;
        if !changed(self.read.raw()) || self.control != normalized(self.read.raw()) {
            return Err(unsupported(
                "FLDCW request lacks its changed-read normalized operand",
            ));
        }
        Ok(())
    }
    pub fn read(&self) -> &ReadReturn {
        &self.read
    }
    pub fn control(&self) -> u16 {
        self.control
    }
}

#[derive(Debug)]
struct LoadOperation {
    request: LoadRequest,
    before: u16,
    after: u16,
    serial: u64,
    exceptions: ExceptionState,
}
#[derive(Debug, Clone)]
pub struct LoadReturn(Rc<LoadOperation>);
impl PartialEq for LoadReturn {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for LoadReturn {}
impl LoadReturn {
    pub fn request(&self) -> &LoadRequest {
        &self.0.request
    }
    pub fn before(&self) -> u16 {
        self.0.before
    }
    pub fn after(&self) -> u16 {
        self.0.after
    }
    pub fn serial(&self) -> u64 {
        self.0.serial
    }
    pub fn validate(&self, request: &LoadRequest) -> Result<(), Stop> {
        request.validate()?;
        if self.0.request != *request
            || self.0.after != request.control()
            || self.0.serial <= request.read().serial()
            || self.0.serial > MAX_OPERATIONS
        {
            return Err(unsupported(
                "FLDCW return has another read, operand or operation serial",
            ));
        }
        // The before word is the actual pre-load state, not necessarily the
        // original read. Flags were retained unchanged by the owned operation.
        wait(self.0.before, self.0.exceptions)
    }
}

#[cfg(test)]
mod tests;
