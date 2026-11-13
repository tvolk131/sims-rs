//! Actual Mac broker/heap/source-cursor integration; synthetic source inputs.
use super::*;
use sims_sim::native_application::{self, mutex};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);
pub(super) struct Authority {
    root: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<sims_kernel::ServerSummary>>>,
}
impl Authority {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "sam-{:x}-{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let server = sims_kernel::Server::bind(&root.join("s"), 0o600).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        Self {
            root,
            stop,
            worker: Some(thread::spawn(move || server.run(signal))),
        }
    }
    pub(super) fn client(&self) -> sims_kernel::Client {
        sims_kernel::Client::connect(&self.root.join("s"), sims_kernel::current_user().unwrap())
            .unwrap()
    }
}
impl Drop for Authority {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            let summary = result.unwrap().unwrap();
            assert_eq!(
                (
                    summary.objects_remaining,
                    summary.handles_remaining,
                    summary.connections_remaining
                ),
                (0, 0, 0)
            );
            assert!(!self.root.join("s").exists());
        }
        let _ = fs::remove_dir(&self.root);
    }
}
pub(super) fn connected<'a>(
    index: &'a Index,
    dirs: &'a Directories,
    authority: &Authority,
    raw: u32,
) -> BootstrapServices<'a> {
    let mut host = services(index, dirs)
        .with_application_version_policy(platform::VersionPolicy::RawDword(raw));
    host.configure_named_object_authority(authority.client())
        .unwrap();
    host
}
pub(super) fn entry(
    c: &mut Construction,
    host: &mut BootstrapServices<'_>,
) -> platform::MutexRequest {
    for _ in 0..4096 {
        if let Some(Call::CreateMutex(request)) = c.pending_call() {
            return request;
        }
        c.step(&mut host.application_host()).unwrap();
    }
    panic!("did not reach real mutex call");
}
pub(super) fn fpu(c: &mut Construction, host: &mut BootstrapServices<'_>) {
    for _ in 0..32 {
        if matches!(
            c.pending_call(),
            Some(Call::ReadControlWord {
                instruction: 0x4eedc7
            })
        ) {
            return;
        }
        c.step(&mut host.application_host()).unwrap();
    }
    panic!("did not reach pending FPU input");
}
fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

#[test]
fn real_mutex_return_reaches_fpu_with_exact_stores_and_acknowledged_temporary_release() {
    let authority = Authority::new();
    for raw in [5, 0x80000005] {
        let (index, dirs) = mounted(&[]);
        let mut host = connected(&index, &dirs, &authority, raw);
        let mut c = Construction::new(input(&mut host)).unwrap();
        let request = entry(&mut c, &mut host);
        let backing = request.name_state().backing.clone();
        let descriptor = request.name_state().owner.clone();
        let before = host.memory.live_bytes();
        assert_eq!(
            host.memory.allocation_bytes(request.wrapper()).unwrap(),
            &[0; 12]
        );
        assert_ne!(
            host.memory.allocation_bytes(c.owner().token).unwrap(),
            request.application_image().data
        );
        fpu(&mut c, &mut host);
        let returned = c.mutex_return().unwrap().clone();
        returned.validate(&request).unwrap();
        assert_ne!(returned.word(), 0);
        assert_eq!(returned.api_error(), None);
        assert_eq!(word(&c.mutex_wrapper().unwrap().data[4..]), returned.word());
        assert_eq!(word(&c.mutex_wrapper().unwrap().data[8..]), 1);
        assert_eq!(word(&c.mutex_singleton().data), request.wrapper() as u32);
        assert_eq!(c.cut(), Some(Cut::MutexReturned));
        assert_eq!(c.owner().data[248], 0);
        assert!(host.memory.allocation_bytes(backing.token).is_err());
        assert_eq!(host.memory.live_bytes(), before - backing.data.len());
        let caller = c
            .retained_callers()
            .iter()
            .find(|o| {
                descriptor.token >= o.token
                    && descriptor.token + 12 <= o.token + o.data.len() as u64
            })
            .unwrap();
        let at = (descriptor.token - caller.token) as usize;
        assert_eq!(&caller.data[at..at + 12], descriptor.data);
        let pcs: Vec<_> = c
            .writes()
            .iter()
            .filter(|w| {
                w.owner == request.wrapper() || w.owner == 0x6f2a5c || w.instruction == 0x4eedb8
            })
            .map(|w| w.instruction)
            .collect();
        assert_eq!(
            pcs,
            vec![
                0x5d94f9,
                0x5d94fd,
                0x5d9511,
                0x5d9516,
                if raw == 5 { 0x4eed54 } else { 0x4eeda2 },
                0x4eedb8
            ]
        );
        host.named_mutex(&c).unwrap().check_live().unwrap();
        assert_eq!(
            host.memory.allocation_bytes(request.wrapper()).unwrap(),
            c.mutex_wrapper().unwrap().data
        );
        returned.close().unwrap();
        for _ in 0..3 {
            c.validate().unwrap();
            returned.validate(&request).unwrap();
        }
        assert!(host.named_mutex(&c).is_err());
        assert!(matches!(
            c.step(&mut host.application_host()),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(host.live_capabilities(), 3);
    }
}

#[test]
fn actual_api_null_keeps_wrapper_publication_and_ready_flag_zero() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut incoming = input(&mut host);
    let name = incoming
        .retained
        .iter_mut()
        .find(|o| o.data.starts_with(b"Global\\Sims\0"))
        .unwrap();
    name.data.fill(0);
    let bytes = b"Global\\invalid\\name\0";
    name.data[..bytes.len()].copy_from_slice(bytes);
    host.memory.synchronize_owner(name).unwrap();
    let mut c = Construction::new(incoming).unwrap();
    let request = entry(&mut c, &mut host);
    fpu(&mut c, &mut host);
    let returned = c.mutex_return().unwrap();
    assert_eq!(returned.api_error(), Some(123));
    assert_eq!(returned.word(), 0);
    assert_eq!(returned.object(), None);
    assert_eq!(&c.mutex_wrapper().unwrap().data[4..], &[0; 8]);
    assert_eq!(word(&c.mutex_singleton().data), request.wrapper() as u32);
    assert!(!c.writes().iter().any(|w| w.instruction == 0x5d9516));
    assert!(
        host.memory
            .allocation_bytes(request.name_state().backing.token)
            .is_err()
    );
    assert!(host.named_mutex(&c).is_err());
    returned.validate(&request).unwrap();
    c.validate().unwrap();
}

#[test]
fn mutex_capability_quota_precedes_dispatch_and_retry_retains_the_source_call() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut c = Construction::new(input(&mut host)).unwrap();
    entry(&mut c, &mut host);
    let next = host.next;
    host.next = CAP_START + MAX_CAPABILITIES;
    let owner = c.owner().clone();
    let writes = c.writes().to_vec();
    let bytes = host.memory.live_bytes();
    for _ in 0..2 {
        assert!(matches!(
            c.step(&mut host.application_host()),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(c.owner(), &owner);
        assert_eq!(c.writes(), writes);
        assert_eq!(host.memory.live_bytes(), bytes);
        assert_eq!(host.live_capabilities(), 2);
        assert!(c.mutex_return().is_none());
        c.validate().unwrap();
    }
    // Restore only the artificial test quota; production IDs never rewind.
    host.next = next;
    fpu(&mut c, &mut host);
    host.named_mutex(&c).unwrap();
}

#[test]
fn same_name_shares_actual_object_across_independent_issuing_heaps() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut first = connected(&index, &dirs, &authority, 5);
    let mut second = connected(&index, &dirs, &authority, 5);
    let mut a = Construction::new(input(&mut first)).unwrap();
    let mut b = Construction::new(input(&mut second)).unwrap();
    entry(&mut a, &mut first);
    entry(&mut b, &mut second);
    fpu(&mut a, &mut first);
    fpu(&mut b, &mut second);
    assert_eq!(a.owner().token, b.owner().token);
    assert_eq!(
        a.mutex_return().unwrap().word(),
        b.mutex_return().unwrap().word()
    );
    assert_ne!(a.mutex_return(), b.mutex_return());
    assert_eq!(
        a.mutex_return().unwrap().object(),
        b.mutex_return().unwrap().object()
    );
    assert!(first.named_mutex(&b).is_err());
    assert!(second.named_mutex(&a).is_err());
    first.named_mutex(&a).unwrap().close().unwrap();
    second.named_mutex(&b).unwrap().check_live().unwrap();
}

#[test]
fn foreign_invocation_heap_swap_and_released_wrapper_fail_before_mutex_effect() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut other = connected(&index, &dirs, &authority, 5);
    let mut c = Construction::new(input(&mut host)).unwrap();
    let mut d = Construction::new(input(&mut other)).unwrap();
    let request = entry(&mut c, &mut host);
    entry(&mut d, &mut other);
    assert!(other.application_host().create_mutex(&request).is_err());
    std::mem::swap(&mut host.memory, &mut other.memory);
    assert!(host.application_host().create_mutex(&request).is_err());
    std::mem::swap(&mut host.memory, &mut other.memory);
    host.memory
        .release(request.wrapper(), Kind::ApplicationMutexWrapper)
        .unwrap();
    assert!(host.application_host().create_mutex(&request).is_err());
    assert_eq!(host.live_capabilities(), 2);
    assert_eq!(other.live_capabilities(), 2);
    c.validate().unwrap();
    d.validate().unwrap();
}

#[test]
fn post_return_heap_tampering_is_rejected_without_changing_immutable_history() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut c = Construction::new(input(&mut host)).unwrap();
    entry(&mut c, &mut host);
    fpu(&mut c, &mut host);
    host.named_mutex(&c).unwrap();
    let mut altered = c.owner().clone();
    altered.data[300] ^= 1;
    host.memory.synchronize_owner(&altered).unwrap();
    assert!(host.named_mutex(&c).is_err());
    c.validate().unwrap();
    host.memory.synchronize_owner(c.owner()).unwrap();
    host.named_mutex(&c).unwrap();
    let mut altered = c.mutex_wrapper().unwrap().clone();
    altered.data[4] ^= 1;
    host.memory.synchronize_owner(&altered).unwrap();
    assert!(host.named_mutex(&c).is_err());
    c.validate().unwrap();
}

#[test]
fn unsupported_authority_profile_retains_one_pending_operation_without_api_null() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut incoming = input(&mut host);
    let name = incoming
        .retained
        .iter_mut()
        .find(|o| o.data.starts_with(b"Global\\Sims\0"))
        .unwrap();
    name.data.fill(0);
    name.data[..8].copy_from_slice(b"Global\\\0");
    host.memory.synchronize_owner(name).unwrap();
    let mut c = Construction::new(incoming).unwrap();
    entry(&mut c, &mut host);
    let writes = c.writes().to_vec();
    for _ in 0..3 {
        assert!(matches!(
            c.step(&mut host.application_host()),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(host.live_capabilities(), 3);
        assert_eq!(c.writes(), writes);
        assert!(c.mutex_return().is_none());
        c.validate().unwrap();
    }
}

struct Gate<'s, 'a> {
    inner: ApplicationHost<'s, 'a>,
    block_free: bool,
    foreign: Option<mutex::Return>,
    creates: usize,
    frees: usize,
}
impl strings::Host for Gate<'_, '_> {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        strings::Host::allocate_string(&mut self.inner, request)
    }
    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        self.frees += 1;
        if self.block_free {
            return Err(Stop::Unavailable(
                "test destructor acknowledgement pending".into(),
            ));
        }
        strings::Host::free_string(&mut self.inner, token)
    }
}
impl native_application::Host for Gate<'_, '_> {
    fn initialize_section(
        &mut self,
        r: &SectionRequest,
    ) -> Result<native_application::InitializedSection, Stop> {
        self.inner.initialize_section(r)
    }
    fn allocate_tree(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.inner.allocate_tree(bytes)
    }
    fn initialize_com(&mut self, r: &com::Request) -> Result<com::Return, Stop> {
        self.inner.initialize_com(r)
    }
    fn query_version(
        &mut self,
        r: &platform::VersionRequest,
    ) -> Result<platform::VersionReturn, Stop> {
        self.inner.query_version(r)
    }
    fn allocate_mutex_wrapper(&mut self, r: &platform::WrapperRequest) -> Result<Allocated, Stop> {
        self.inner.allocate_mutex_wrapper(r)
    }
    fn create_mutex(&mut self, r: &platform::MutexRequest) -> Result<mutex::Return, Stop> {
        self.creates += 1;
        match &self.foreign {
            Some(returned) => Ok(returned.clone()),
            None => self.inner.create_mutex(r),
        }
    }
}

#[test]
fn destructor_waits_for_acknowledgement_and_replay_never_repeats_ipc_or_free() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut c = Construction::new(input(&mut host)).unwrap();
    let request = entry(&mut c, &mut host);
    let mut gate = Gate {
        inner: host.application_host(),
        block_free: true,
        foreign: None,
        creates: 0,
        frees: 0,
    };
    c.step(&mut gate).unwrap(); // actual reply staged
    c.step(&mut gate).unwrap(); // wrapper stores
    c.step(&mut gate).unwrap(); // singleton/EH publication
    let writes = c.writes().to_vec();
    let owner = c.owner().clone();
    for _ in 0..2 {
        assert!(matches!(c.step(&mut gate), Err(Stop::Unavailable(_))));
        assert_eq!(c.writes(), writes);
        assert_eq!(c.owner(), &owner);
        c.validate().unwrap();
        assert_eq!(gate.creates, 1);
        assert!(
            gate.inner
                .services
                .memory
                .allocation_bytes(request.name_state().backing.token)
                .is_ok()
        );
    }
    gate.block_free = false;
    c.step(&mut gate).unwrap();
    c.step(&mut gate).unwrap();
    assert_eq!((gate.creates, gate.frees), (1, 3));
    for _ in 0..3 {
        c.validate().unwrap();
    }
    assert_eq!((gate.creates, gate.frees), (1, 3));
    assert!(matches!(
        c.pending_call(),
        Some(Call::ReadControlWord { .. })
    ));
    drop(gate);
    host.named_mutex(&c).unwrap();
}

#[test]
fn foreign_successful_return_is_retained_before_validation_and_never_requested_again() {
    let authority = Authority::new();
    let (index, dirs) = mounted(&[]);
    let mut host = connected(&index, &dirs, &authority, 5);
    let mut other = connected(&index, &dirs, &authority, 5);
    let mut c = Construction::new(input(&mut host)).unwrap();
    let mut d = Construction::new(input(&mut other)).unwrap();
    entry(&mut c, &mut host);
    entry(&mut d, &mut other);
    fpu(&mut d, &mut other);
    let mut gate = Gate {
        inner: host.application_host(),
        block_free: false,
        foreign: d.mutex_return().cloned(),
        creates: 0,
        frees: 0,
    };
    let owner = c.owner().clone();
    let writes = c.writes().to_vec();
    for _ in 0..3 {
        assert!(matches!(c.step(&mut gate), Err(Stop::Unsupported(_))));
        assert_eq!(gate.creates, 1);
        assert_eq!(c.owner(), &owner);
        assert_eq!(c.writes(), writes);
        assert!(c.validate().is_err());
    }
    gate.foreign.as_ref().unwrap().check_live().unwrap();
}
