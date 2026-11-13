use super::*;
use crate::native_world::resources::handle;

mod population;

#[derive(Debug, Clone, PartialEq, Eq)]
enum HandleCall {
    Allocate(u32),
    Size(u32),
    String(strings::Request),
    Free(Owner),
}

struct HandleHost {
    file: OpenHost,
    canonical: Owner,
    next: u64,
    live: BTreeMap<u64, usize>,
    calls: Vec<HandleCall>,
    handle_reply: Option<Allocated>,
    string_reply: Option<(usize, Allocated)>,
    string_allocations: usize,
    size_reply: handle::FileSize,
    deny: bool,
    deny_free: bool,
}

impl HandleHost {
    fn available(&self) -> Result<(), Stop> {
        if self.deny {
            Err(Stop::Unavailable("handle service pending".into()))
        } else {
            Ok(())
        }
    }

    fn allocate(&mut self, bytes: u32, reply: Option<Allocated>) -> Allocated {
        let token = self.next;
        self.next += u64::from((bytes + 15) & !15);
        let reply = reply.unwrap_or_else(|| Allocated::Bytes {
            token,
            data: vec![0x7d; bytes as usize],
        });
        if let Allocated::Bytes { token, data } = &reply {
            self.live.insert(*token, data.len());
        }
        reply
    }
}

impl handle::Host for HandleHost {
    fn allocate_handle(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.available()?;
        self.calls.push(HandleCall::Allocate(bytes));
        let reply = self.handle_reply.take();
        Ok(self.allocate(bytes, reply))
    }

    fn file_size(&mut self, capability: u32) -> Result<handle::FileSize, Stop> {
        self.available()?;
        assert!(self.file.opened);
        assert_eq!(capability, 0x41000000);
        self.calls.push(HandleCall::Size(capability));
        Ok(self.size_reply)
    }

    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        self.available()?;
        self.string_allocations += 1;
        self.calls.push(HandleCall::String(request));
        let reply = if self
            .string_reply
            .as_ref()
            .is_some_and(|(index, _)| *index == self.string_allocations)
        {
            Some(self.string_reply.take().unwrap().1)
        } else {
            None
        };
        Ok(self.allocate(request.bytes(), reply))
    }

    fn free_string(&mut self, backing: &Owner) -> Result<(), Stop> {
        self.available()?;
        if self.deny_free {
            return Err(Stop::Unavailable("handle name destructor pending".into()));
        }
        assert_eq!(self.live.remove(&backing.token), Some(backing.data.len()));
        if backing.token == self.canonical.token {
            assert_eq!(*backing, self.canonical);
        }
        self.calls.push(HandleCall::Free(backing.clone()));
        Ok(())
    }
}

fn handle_construction() -> (handle::Construction, HandleHost) {
    let (mut execution, mut file) = opening();
    reach(&mut execution, &mut file, Cut::Returned);
    let opened = execution.into_state().unwrap();
    let canonical = opened.context().name.backing.clone();
    let host = HandleHost {
        file,
        live: BTreeMap::from([(canonical.token, canonical.data.len())]),
        canonical,
        next: 0x75000000,
        calls: vec![],
        handle_reply: None,
        string_reply: None,
        string_allocations: 0,
        size_reply: handle::FileSize::Bytes(2271104),
        deny: false,
        deny_free: false,
    };
    (handle::Construction::new(opened).unwrap(), host)
}

fn reach_handle(execution: &mut handle::Construction, host: &mut HandleHost, cut: handle::Cut) {
    for _ in 0..140 {
        let restored: handle::Construction =
            serde_json::from_slice(&serde_json::to_vec(execution).unwrap()).unwrap();
        assert_eq!(*execution, restored);
        restored.validate().unwrap();
        if execution.cut() == Some(cut) {
            return;
        }
        execution.step(host).unwrap();
    }
    panic!("handle cut {cut:?} not reached");
}

fn stop_handle(execution: &mut handle::Construction, host: &mut HandleHost) {
    for _ in 0..140 {
        if execution.step(host).is_err() {
            return;
        }
    }
    panic!("handle did not stop");
}

fn assert_handle_sticky(execution: &mut handle::Construction, host: &mut HandleHost) {
    let before = execution.clone();
    let calls = host.calls.clone();
    for _ in 0..3 {
        assert!(execution.step(host).is_err());
        assert_eq!(*execution, before);
        assert_eq!(host.calls, calls);
        let restored: handle::Construction =
            serde_json::from_slice(&serde_json::to_vec(execution).unwrap()).unwrap();
        assert_eq!(*execution, restored);
    }
    assert!(execution.clone().into_state().is_err());
    assert!(host.file.opened);
}

#[test]
fn handle_composes_complete_owners_and_publishes_only_after_canonical_release() {
    let (mut execution, mut host) = handle_construction();
    let opened = execution.opened().clone();
    let resources = execution.resources().clone();
    let mut cuts = vec![];
    for _ in 0..140 {
        if let Some(cut) = execution.step(&mut host).unwrap() {
            cuts.push(cut);
        }
        assert_eq!(execution.opened(), &opened);
        if execution.cut() != Some(handle::Cut::Published) {
            assert_eq!(execution.resources(), &resources);
        }
        if execution.cut() == Some(handle::Cut::Constructed) {
            assert_eq!(
                execution.name().unwrap().backing.data,
                [&[0][..], &[0x7d; 7]].concat()
            );
        }
        if execution.cut() == Some(handle::Cut::Published) {
            break;
        }
    }
    assert_eq!(
        cuts,
        [
            handle::Cut::Allocated,
            handle::Cut::SizeReturned,
            handle::Cut::Constructed,
            handle::Cut::NameAssigned,
            handle::Cut::NameReleased,
            handle::Cut::Published
        ]
    );
    assert_eq!(
        host.calls
            .iter()
            .map(|call| match call {
                HandleCall::Allocate(bytes) => format!("allocate{bytes}"),
                HandleCall::Size(capability) => format!("size{capability:#x}"),
                HandleCall::String(request) => format!("string{request:?}"),
                HandleCall::Free(owner) => format!("free{}", owner.data.len()),
            })
            .collect::<Vec<_>>(),
        [
            "allocate36",
            "size0x41000000",
            "stringConstruct(8)",
            "stringGrow(24)",
            "free8",
            "free24"
        ]
    );
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    assert_eq!(state.owner().data.len(), 36);
    assert_eq!(&state.owner().data[30..32], [0x7d, 0x7d]);
    assert_eq!(state.name().bytes().unwrap(), b".\\gamedata\\lightmap.dat");
    assert_eq!(&state.resources().data[..96], &resources.data[..96]);
    assert_eq!(
        &state.resources().data[96..],
        (state.owner().token as u32).to_le_bytes()
    );
    assert_eq!(host.live.len(), 2);
    assert!(!host.live.contains_key(&host.canonical.token));
    assert!(host.file.opened);
}

#[test]
fn handle_uses_real_size_reply_after_handle_allocation_without_fabricating_payload() {
    let (mut execution, mut host) = handle_construction();
    host.size_reply = handle::FileSize::Bytes(73);
    reach_handle(&mut execution, &mut host, handle::Cut::SizeReturned);
    assert_eq!(
        host.calls,
        [HandleCall::Allocate(36), HandleCall::Size(0x41000000)]
    );
    assert_eq!(execution.owner().unwrap().data, [0x7d; 36]);
    reach_handle(&mut execution, &mut host, handle::Cut::Published);
    assert_eq!(
        &execution.owner().unwrap().data[16..20],
        73_u32.to_le_bytes()
    );
}

#[test]
fn unavailable_handle_services_are_retryable_without_duplicate_completed_calls() {
    let (mut execution, mut host) = handle_construction();
    let mut blocked = 0;
    for _ in 0..160 {
        let before = execution.clone();
        let calls = host.calls.clone();
        host.deny = true;
        match execution.step(&mut host) {
            Ok(_) => {}
            Err(Stop::Unavailable(_)) => {
                blocked += 1;
                assert_eq!(execution, before);
                assert_eq!(host.calls, calls);
                host.deny = false;
                execution.step(&mut host).unwrap();
            }
            Err(error) => panic!("unexpected {error:?}"),
        }
        if execution.cut() == Some(handle::Cut::Published) {
            break;
        }
    }
    assert_eq!(blocked, 6);
    assert!(host.file.opened);
}

#[test]
fn invalid_handle_allocations_cannot_reallocate_query_size_or_publish() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x75000000,
            data: vec![0x7d; 35],
        },
        Allocated::Bytes {
            token: u64::from(u32::MAX),
            data: vec![0x7d; 36],
        },
    ] {
        let (mut execution, mut host) = handle_construction();
        host.handle_reply = Some(reply);
        stop_handle(&mut execution, &mut host);
        assert_eq!(host.calls, [HandleCall::Allocate(36)]);
        assert_handle_sticky(&mut execution, &mut host);
    }
}

#[test]
fn handle_allocation_cannot_alias_retained_file_manager_canonical_or_resource_owner() {
    for kind in 0..4 {
        let (mut execution, mut host) = handle_construction();
        let token = match kind {
            0 => execution.opened().owner().token,
            1 => execution.opened().context().manager.owner.token,
            2 => execution.opened().context().name.backing.token,
            _ => execution.resources().token,
        };
        host.handle_reply = Some(Allocated::Bytes {
            token,
            data: vec![0x7d; 36],
        });
        stop_handle(&mut execution, &mut host);
        assert_eq!(host.calls, [HandleCall::Allocate(36)]);
        assert_handle_sticky(&mut execution, &mut host);
    }
}

#[test]
fn failed_and_unqualified_size_replies_remain_pending_without_repeat_queries() {
    for reply in [
        handle::FileSize::Failed,
        handle::FileSize::Bytes(u64::MAX),
        handle::FileSize::Bytes(u64::from(u32::MAX)),
    ] {
        let (mut execution, mut host) = handle_construction();
        host.size_reply = reply;
        stop_handle(&mut execution, &mut host);
        assert_eq!(
            host.calls,
            [HandleCall::Allocate(36), HandleCall::Size(0x41000000)]
        );
        assert_eq!(execution.size(), None);
        assert_handle_sticky(&mut execution, &mut host);
    }
}

#[test]
fn invalid_default_and_growth_replies_do_not_repeat_allocations_or_lose_ownership() {
    for index in 1..=2 {
        for kind in 0..3 {
            let (mut execution, mut host) = handle_construction();
            let bytes = if index == 1 { 8 } else { 24 };
            let reply = match kind {
                0 => Allocated::Null,
                1 => Allocated::Bytes {
                    token: 0x76000000,
                    data: vec![0x7d; bytes - 1],
                },
                _ => Allocated::Bytes {
                    token: execution.opened().owner().token,
                    data: vec![0x7d; bytes],
                },
            };
            host.string_reply = Some((index, reply));
            stop_handle(&mut execution, &mut host);
            assert_eq!(host.string_allocations, index);
            assert_handle_sticky(&mut execution, &mut host);
        }
    }
}

#[test]
fn failed_handle_growth_release_retains_old_new_and_canonical_backings() {
    let (mut execution, mut host) = handle_construction();
    reach_handle(&mut execution, &mut host, handle::Cut::Constructed);
    host.deny_free = true;
    stop_handle(&mut execution, &mut host);
    execution.validate().unwrap();
    assert_eq!(host.live.len(), 4);
    assert_handle_sticky(&mut execution, &mut host);
    host.deny_free = false;
    reach_handle(&mut execution, &mut host, handle::Cut::Published);
}

#[test]
fn failed_canonical_release_cannot_publish_resource_handle_or_drop_file() {
    let (mut execution, mut host) = handle_construction();
    reach_handle(&mut execution, &mut host, handle::Cut::NameAssigned);
    let resources = execution.resources().clone();
    host.deny_free = true;
    stop_handle(&mut execution, &mut host);
    execution.validate().unwrap();
    assert_eq!(execution.resources(), &resources);
    assert_eq!(host.live.len(), 3);
    assert_handle_sticky(&mut execution, &mut host);
    host.deny_free = false;
    reach_handle(&mut execution, &mut host, handle::Cut::Published);
}

#[test]
fn retired_opening_frames_do_not_remain_duplicate_mutable_owners() {
    let (mut execution, mut host) = handle_construction();
    host.handle_reply = Some(Allocated::Bytes {
        token: 0x61007660,
        data: vec![0x7d; 36],
    });
    reach_handle(&mut execution, &mut host, handle::Cut::Published);
    assert_eq!(execution.owner().unwrap().token, 0x61007660);
    execution.into_state().unwrap().validate().unwrap();
}

#[test]
fn restored_handle_rejects_changed_incoming_descriptor_or_early_publication() {
    let (mut execution, mut host) = handle_construction();
    reach_handle(&mut execution, &mut host, handle::Cut::Allocated);
    let value = serde_json::to_value(&execution).unwrap();
    let mut broken = value.clone();
    broken["owner"]["data"][0] = 1.into();
    broken["name"]["Bare"]["data"][0] = 1.into();
    let broken: handle::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
    let mut broken = value;
    broken["resources"]["data"][96] = 1.into();
    let broken: handle::Construction = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
    reach_handle(&mut execution, &mut host, handle::Cut::Published);
    let mut broken = serde_json::to_value(execution.into_state().unwrap()).unwrap();
    broken["execution"]["owner"]["data"] = serde_json::json!([0]);
    let broken: handle::State = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
}
