use super::*;
use crate::native_world::resources::opening::{
    self, Cut, Host, LocaleReply, LocaleRequest, OpenRequest, Opened, Request, Seeked,
};
use std::collections::VecDeque;

mod handles;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OpenCall {
    Allocate(Request),
    Free(Owner),
    Locale(LocaleRequest),
    Open(OpenRequest),
    Seek(u32),
}

struct OpenHost {
    next: u64,
    calls: Vec<OpenCall>,
    live: BTreeMap<u64, usize>,
    allocation_reply: Option<(usize, Allocated)>,
    allocations: usize,
    locale_replies: VecDeque<LocaleReply>,
    open_reply: Opened,
    seek_reply: Seeked,
    opened: bool,
    deny: bool,
    deny_free: bool,
}

impl OpenHost {
    fn new() -> Self {
        Self {
            next: 0x74000000,
            calls: vec![],
            live: BTreeMap::new(),
            allocation_reply: None,
            allocations: 0,
            locale_replies: VecDeque::new(),
            open_reply: Opened::File(0x41000000),
            seek_reply: Seeked::Position(0),
            opened: false,
            deny: false,
            deny_free: false,
        }
    }

    fn available(&self) -> Result<(), Stop> {
        if self.deny {
            Err(Stop::Unavailable("opening service pending".into()))
        } else {
            Ok(())
        }
    }
}

impl Host for OpenHost {
    fn allocate_wide(&mut self, request: Request) -> Result<Allocated, Stop> {
        self.available()?;
        self.allocations += 1;
        self.calls.push(OpenCall::Allocate(request));
        let reply = if self
            .allocation_reply
            .as_ref()
            .is_some_and(|(index, _)| *index == self.allocations)
        {
            self.allocation_reply.take().unwrap().1
        } else {
            let token = self.next;
            self.next += u64::from((request.bytes() + 15) & !15);
            Allocated::Bytes {
                token,
                data: vec![0xa5; request.bytes() as usize],
            }
        };
        if let Allocated::Bytes { token, data } = &reply {
            self.live.insert(*token, data.len());
        }
        Ok(reply)
    }

    fn free_wide(&mut self, backing: &Owner) -> Result<(), Stop> {
        self.available()?;
        if self.deny_free {
            return Err(Stop::Unavailable("wide destructor pending".into()));
        }
        assert_eq!(self.live.remove(&backing.token), Some(backing.data.len()));
        self.calls.push(OpenCall::Free(backing.clone()));
        Ok(())
    }

    fn locale(&mut self, request: LocaleRequest) -> Result<LocaleReply, Stop> {
        self.available()?;
        self.calls.push(OpenCall::Locale(request));
        Ok(self
            .locale_replies
            .pop_front()
            .unwrap_or_else(|| LocaleReply::Value(b"C".to_vec())))
    }

    fn open_readonly(&mut self, request: &OpenRequest) -> Result<Opened, Stop> {
        self.available()?;
        assert!(!self.opened);
        self.calls.push(OpenCall::Open(request.clone()));
        self.opened = matches!(self.open_reply, Opened::File(capability) if capability != 0 && capability != u32::MAX);
        Ok(self.open_reply)
    }

    fn seek_current(&mut self, capability: u32) -> Result<Seeked, Stop> {
        self.available()?;
        assert!(self.opened);
        assert_eq!(capability, 0x41000000);
        self.calls.push(OpenCall::Seek(capability));
        Ok(self.seek_reply)
    }
}

fn closed_file() -> file::State {
    let (mut execution, mut host) = file_construction();
    finish_file(&mut execution, &mut host);
    execution.into_state().unwrap()
}

fn closed_file_with_path(inputs: path::Inputs) -> file::State {
    let mut input = incoming();
    input.directory_descriptor.token = input.basename_descriptor.token + 12;
    let mut names = names::Construction::new(path_state(inputs), input).unwrap();
    let mut host = NameHost::new();
    for _ in 0..200 {
        if names.step(&mut host).unwrap() == Some(names::Cut::FileAllocationEntry) {
            let mut execution = file::Construction::new(names.into_state().unwrap()).unwrap();
            finish_file(&mut execution, &mut FileHost::new());
            return execution.into_state().unwrap();
        }
    }
    panic!("alternate path did not reach the genuine file constructor");
}

fn open_incoming(closed: &file::State) -> opening::Incoming {
    opening::Incoming {
        output: Owner {
            token: 0x61007bc0,
            data: vec![0x37; 12],
        },
        local: Owner {
            token: 0x61007a70,
            data: vec![0x59; 12],
        },
        replacement: Owner {
            token: closed.context.retired_prefix.token,
            data: vec![0x83; 12],
        },
        buffer: Owner {
            token: 0x61007660,
            data: vec![0x91; 1024],
        },
        buffer_units: 512,
        encoding: Owner {
            token: 0x6d28a8,
            data: 0xfffffffd_u32.to_le_bytes().to_vec(),
        },
        wide_dispatch: Owner {
            token: 0x6fe5d8,
            data: 0x7b60ae9c_u32.to_le_bytes().to_vec(),
        },
        arguments: [1, 2, 1],
    }
}

fn opening() -> (opening::Execution, OpenHost) {
    let closed = closed_file();
    let incoming = open_incoming(&closed);
    (
        opening::Execution::new(closed, incoming).unwrap(),
        OpenHost::new(),
    )
}

fn reach(execution: &mut opening::Execution, host: &mut OpenHost, cut: Cut) {
    for _ in 0..100 {
        let restored: opening::Execution =
            serde_json::from_slice(&serde_json::to_vec(execution).unwrap()).unwrap();
        assert_eq!(*execution, restored);
        restored.validate().unwrap();
        if execution.cut() == Some(cut) {
            return;
        }
        execution.step(host).unwrap();
    }
    panic!("opening cut {cut:?} not reached");
}

fn stop(execution: &mut opening::Execution, host: &mut OpenHost) -> Stop {
    for _ in 0..100 {
        if let Err(error) = execution.step(host) {
            return error;
        }
    }
    panic!("opening did not stop");
}

fn assert_sticky(execution: &mut opening::Execution, host: &mut OpenHost) {
    let before = execution.clone();
    let calls = host.calls.clone();
    for _ in 0..3 {
        assert!(execution.step(host).is_err());
        assert_eq!(*execution, before);
        assert_eq!(host.calls, calls);
        let restored: opening::Execution =
            serde_json::from_slice(&serde_json::to_vec(execution).unwrap()).unwrap();
        assert_eq!(*execution, restored);
    }
    assert!(execution.clone().into_state().is_err());
}

#[test]
fn readonly_open_preserves_full_owners_and_service_order_with_live_capability() {
    let (mut execution, mut host) = opening();
    let context = execution.context().clone();
    let before = execution.owner().clone();
    let mut cuts = vec![execution.cut().unwrap()];
    for _ in 0..100 {
        let restored: opening::Execution =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(execution, restored);
        restored.validate().unwrap();
        if let Some(cut) = execution.step(&mut host).unwrap() {
            cuts.push(cut);
        }
        assert_eq!(execution.context(), &context);
        if execution.cut() == Some(Cut::Returned) {
            break;
        }
    }
    assert_eq!(
        cuts,
        [
            Cut::OpenEntry,
            Cut::ConversionEntry,
            Cut::LocalInput,
            Cut::LocalReady,
            Cut::BoundedConversionEntry,
            Cut::BoundedConversionReturn,
            Cut::LocalAssigned,
            Cut::OutputCopied,
            Cut::LocalReleased,
            Cut::ReplacementInput,
            Cut::ReplacementReady,
            Cut::ShortcutChecked,
            Cut::ReadonlyEntry,
            Cut::ReadonlyReturn,
            Cut::ReplacementReleased,
            Cut::OutputReleased,
            Cut::FieldsPublished,
            Cut::Returned
        ]
    );
    assert_eq!(
        host.calls
            .iter()
            .map(|call| match call {
                OpenCall::Allocate(request) => format!("allocate{request:?}"),
                OpenCall::Free(owner) => format!("free{}", owner.data.len()),
                OpenCall::Locale(request) => format!("locale{request:?}"),
                OpenCall::Open(_) => "open".into(),
                OpenCall::Seek(_) => "seek".into(),
            })
            .collect::<Vec<_>>(),
        [
            "allocateConstruct(16)",
            "localeQuery",
            "localeDefault",
            "allocateGrow(48)",
            "free16",
            "allocateConstruct(48)",
            "free48",
            "allocateConstruct(16)",
            "open",
            "free16",
            "free48",
            "seek"
        ]
    );
    let request = host
        .calls
        .iter()
        .find_map(|call| {
            if let OpenCall::Open(request) = call {
                Some(request)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        *request,
        OpenRequest {
            filename: "./GameData/lightmap.dat".encode_utf16().collect(),
            access: 0x80000000,
            sharing: 1,
            security: 0,
            disposition: 3,
            flags: 0,
            template: 0
        }
    );
    assert!(host.opened && host.live.is_empty());
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    assert_eq!(state.capability(), 0x41000000);
    let mut expected = before;
    expected.data[0x18] = 1;
    for (offset, value) in [(0x1c, 0x41000000_u32), (0x20, 1), (0x24, 2), (0x28, 1)] {
        expected.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(state.owner(), &expected);
}

#[test]
fn conversion_writes_no_terminator_and_wide_strings_preserve_poisoned_tails() {
    let (mut execution, mut host) = opening();
    reach(&mut execution, &mut host, Cut::BoundedConversionEntry);
    assert_eq!(execution.buffer().data, vec![0x91; 1024]);
    assert_eq!(execution.conversion_units(), 512);
    let default = execution.local_backing().unwrap();
    assert_eq!(&default.data[..2], [0, 0]);
    assert_eq!(&default.data[2..], [0xa5; 14]);
    reach(&mut execution, &mut host, Cut::BoundedConversionReturn);
    let widened: Vec<_> = "./GameData/lightmap.dat"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(&execution.buffer().data[..46], widened);
    assert_eq!(&execution.buffer().data[46..], [0x91; 978]);
    assert_eq!(execution.conversion_units(), 23);
    reach(&mut execution, &mut host, Cut::LocalAssigned);
    let backing = execution.local_backing().unwrap();
    assert_eq!(&backing.data[..46], widened);
    assert_eq!(&backing.data[46..], [0, 0]);
    reach(&mut execution, &mut host, Cut::Returned);
    assert_eq!(&execution.buffer().data[46..], [0x91; 978]);
}

#[test]
fn cached_c_encoding_does_not_repeat_locale_callbacks() {
    let closed = closed_file();
    let mut incoming = open_incoming(&closed);
    incoming.encoding.data.fill(0);
    let mut execution = opening::Execution::new(closed, incoming).unwrap();
    let mut host = OpenHost::new();
    reach(&mut execution, &mut host, Cut::Returned);
    assert!(
        !host
            .calls
            .iter()
            .any(|call| matches!(call, OpenCall::Locale(_)))
    );
}

#[test]
fn unsupported_small_and_unbounded_conversions_stop_before_host_services() {
    for filename in [b"a.dat".to_vec(), vec![b'x'; 300]] {
        let closed = closed_file_with_path(path::Inputs {
            directory: None,
            filename: Some(filename),
        });
        let incoming = open_incoming(&closed);
        assert!(opening::Execution::new(closed, incoming).is_err());
    }
}

#[test]
fn actual_shortcut_path_stops_without_inventing_resolution_or_opening() {
    let closed = closed_file_with_path(path::Inputs {
        directory: Some(b"GameData/".to_vec()),
        filename: Some(b"lightmap.lnk".to_vec()),
    });
    let incoming = open_incoming(&closed);
    let mut execution = opening::Execution::new(closed, incoming).unwrap();
    let mut host = OpenHost::new();
    assert!(matches!(
        stop(&mut execution, &mut host),
        Stop::Unsupported(_)
    ));
    assert_eq!(host.live.len(), 2);
    assert!(!host.opened);
    assert_sticky(&mut execution, &mut host);
}

#[test]
fn restored_cached_encoding_cannot_invent_locale_initialization_work() {
    let closed = closed_file();
    let mut incoming = open_incoming(&closed);
    incoming.encoding.data.fill(0);
    let mut execution = opening::Execution::new(closed, incoming).unwrap();
    reach(
        &mut execution,
        &mut OpenHost::new(),
        Cut::BoundedConversionEntry,
    );
    let mut value = serde_json::to_value(execution).unwrap();
    value["phase"] = "LocaleQuery".into();
    let broken: opening::Execution = serde_json::from_value(value).unwrap();
    assert!(broken.validate().is_err());
}

#[test]
fn unavailable_services_leave_every_callback_boundary_retryable_without_duplicate_effects() {
    let (mut execution, mut host) = opening();
    let mut blocked = 0;
    for _ in 0..120 {
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
        if execution.cut() == Some(Cut::Returned) {
            break;
        }
    }
    assert_eq!(blocked, 12);
    assert!(host.opened && host.live.is_empty());
}

#[test]
fn null_or_malformed_wide_allocations_are_retained_without_reallocation() {
    for index in 1..=4 {
        let bytes = if index == 1 || index == 4 { 16 } else { 48 };
        for reply in [
            Allocated::Null,
            Allocated::Bytes {
                token: 0x75000000,
                data: vec![0xa5; bytes - 1],
            },
            Allocated::Bytes {
                token: 0x75000001,
                data: vec![0xa5; bytes],
            },
            Allocated::Bytes {
                token: 0xfffffff0,
                data: vec![0xa5; bytes],
            },
        ] {
            let (mut execution, mut host) = opening();
            host.allocation_reply = Some((index, reply));
            stop(&mut execution, &mut host);
            assert_eq!(host.allocations, index);
            assert_sticky(&mut execution, &mut host);
        }
    }
}

#[test]
fn allocator_cannot_alias_file_canonical_name_stack_or_other_wide_backings() {
    for index in 1..=4 {
        let bytes = if index == 1 || index == 4 { 16 } else { 48 };
        for kind in 0..4 {
            let (mut execution, mut host) = opening();
            let token = match kind {
                0 => execution.owner().token,
                1 => execution.context().name.backing.token,
                2 => execution.buffer().token,
                _ => {
                    if index == 1 {
                        execution.output_descriptor().token
                    } else {
                        0x74000000
                    }
                }
            };
            host.allocation_reply = Some((
                index,
                Allocated::Bytes {
                    token,
                    data: vec![0xa5; bytes],
                },
            ));
            if index >= 3 && kind == 3 {
                continue;
            }
            stop(&mut execution, &mut host);
            assert_eq!(host.allocations, index);
            assert_sticky(&mut execution, &mut host);
        }
    }
}

#[test]
fn failed_locale_replies_are_not_requeried_or_replaced_by_ready_encoding() {
    for index in 0..2 {
        for reply in [
            LocaleReply::Failed,
            LocaleReply::Value(b"en_US.UTF-8".to_vec()),
        ] {
            let (mut execution, mut host) = opening();
            for _ in 0..index {
                host.locale_replies
                    .push_back(LocaleReply::Value(b"C".to_vec()));
            }
            host.locale_replies.push_back(reply);
            stop(&mut execution, &mut host);
            assert_eq!(
                host.calls
                    .iter()
                    .filter(|call| matches!(call, OpenCall::Locale(_)))
                    .count(),
                index + 1
            );
            assert_eq!(execution.encoding().data, [0; 4]);
            assert_eq!(execution.buffer().data, [0x91; 1024]);
            assert_sticky(&mut execution, &mut host);
        }
    }
}

#[test]
fn failed_open_replies_do_not_reopen_seek_release_or_publish_success() {
    for reply in [Opened::Failed, Opened::File(0), Opened::File(u32::MAX)] {
        let (mut execution, mut host) = opening();
        host.open_reply = reply;
        stop(&mut execution, &mut host);
        assert_eq!(
            host.calls
                .iter()
                .filter(|call| matches!(call, OpenCall::Open(_)))
                .count(),
            1
        );
        assert_eq!(host.live.len(), 2);
        assert_eq!(execution.owner().data[0x18], 0);
        assert_eq!(execution.capability(), None);
        assert_sticky(&mut execution, &mut host);
    }
}

#[test]
fn failed_seek_keeps_capability_and_cannot_reopen_or_repeat_position_query() {
    for reply in [
        Seeked::Failed,
        Seeked::Position(u64::MAX),
        Seeked::Position(u64::from(u32::MAX)),
    ] {
        let (mut execution, mut host) = opening();
        host.seek_reply = reply;
        stop(&mut execution, &mut host);
        assert!(host.opened && host.live.is_empty());
        assert_eq!(execution.capability(), Some(0x41000000));
        assert_sticky(&mut execution, &mut host);
    }
}

#[test]
fn failed_growth_release_preserves_both_backings_and_prepared_terminator() {
    let (mut execution, mut host) = opening();
    reach(&mut execution, &mut host, Cut::BoundedConversionReturn);
    host.deny_free = true;
    assert!(matches!(
        stop(&mut execution, &mut host),
        Stop::Unavailable(_)
    ));
    execution.validate().unwrap();
    assert_eq!(host.live.len(), 2);
    assert_eq!(execution.local_backing().unwrap().data.len(), 16);
    assert_sticky(&mut execution, &mut host);
    host.deny_free = false;
    reach(&mut execution, &mut host, Cut::Returned);
}

#[test]
fn failed_post_open_destruction_keeps_real_capability_and_pending_temporary() {
    for cut in [Cut::ReadonlyReturn, Cut::ReplacementReleased] {
        let (mut execution, mut host) = opening();
        reach(&mut execution, &mut host, cut);
        host.deny_free = true;
        stop(&mut execution, &mut host);
        execution.validate().unwrap();
        assert!(host.opened && !host.live.is_empty());
        assert_eq!(execution.owner().data[0x18], 0);
        assert_sticky(&mut execution, &mut host);
        host.deny_free = false;
        reach(&mut execution, &mut host, Cut::Returned);
    }
}

#[test]
fn incoming_frames_retire_constructor_images_but_cannot_alias_permanent_owners() {
    let closed = closed_file();
    let incoming = open_incoming(&closed);
    assert_eq!(
        incoming.replacement.token,
        closed.context.retired_prefix.token
    );
    assert_ne!(
        incoming.replacement.data[..4],
        closed.context.retired_prefix.data
    );
    opening::Execution::new(closed.clone(), incoming.clone()).unwrap();
    for kind in 0..7 {
        let mut broken = incoming.clone();
        match kind {
            0 => broken.replacement.token = closed.owner.token,
            1 => broken.replacement.token = closed.context.name.owner.token,
            2 => broken.local.token = broken.output.token,
            3 => broken.encoding.token = broken.buffer.token,
            4 => broken.wide_dispatch.data.fill(0),
            5 => broken.buffer_units = 511,
            _ => broken.arguments[0] = 2,
        }
        assert!(opening::Execution::new(closed.clone(), broken).is_err());
    }
}

#[test]
fn restored_opening_rejects_corrupted_complete_images_without_panicking() {
    let (mut execution, mut host) = opening();
    reach(&mut execution, &mut host, Cut::LocalAssigned);
    let value = serde_json::to_value(&execution).unwrap();
    for field in ["owner", "encoding", "buffer"] {
        let mut broken = value.clone();
        broken[field]["data"] = serde_json::json!([0]);
        let broken: opening::Execution = serde_json::from_value(broken).unwrap();
        assert!(broken.validate().is_err());
    }
    let mut broken = value;
    broken["local"]["backing"]["data"][47] = 1.into();
    let broken: opening::Execution = serde_json::from_value(broken).unwrap();
    assert!(broken.validate().is_err());
}
