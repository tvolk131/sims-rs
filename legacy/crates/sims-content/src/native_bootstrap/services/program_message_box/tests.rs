//! Protocol controls provide no display/user/Windows/native/guest qualification.
//! Modules below come from real descriptor reads of a wholly authored PE32;
//! these controls neither seed ModuleOrigin nor read game/private assets.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use super::*;
use crate::native_bootstrap::executable;
use std::{
    ffi::CString,
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    sync::atomic::AtomicU64,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        for _ in 0..128 {
            let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("sims-messagebox-protocol-{serial}"));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    let fixture = Self(path);
                    let mut file = OpenOptions::new().write(true).create_new(true)
                        .mode(0o600).open(fixture.0.join("authored.exe")).unwrap();
                    file.write_all(&authored_pe()).unwrap();
                    return fixture;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(error) => panic!("private authored fixture: {error}"),
            }
        }
        panic!("bounded exclusive fixture names exhausted");
    }

    fn module(&self) -> Module { executable::read(&self.0.join("authored.exe")).unwrap() }
}

impl Drop for Fixture {
    fn drop(&mut self) { fs::remove_dir_all(&self.0).unwrap(); }
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn authored_pe() -> Vec<u8> {
    // Independent authored I386 image: one normal aligned section, no imports.
    let mut bytes = vec![0; 0x400];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 0x84, 0x14c);
    put16(&mut bytes, 0x86, 1);
    put16(&mut bytes, 0x94, 224);
    put16(&mut bytes, 0x96, 0x102);
    let o = 0x98;
    put16(&mut bytes, o, 0x10b);
    for (offset, value) in [
        (4, 0x200), (12, 0x100), (16, 0x1000), (20, 0x1000),
        (24, 0x1000), (28, 0x0040_0000), (32, 0x1000), (36, 0x200),
        (56, 0x2000), (60, 0x200), (72, 0x10_0000), (76, 0x1000),
        (80, 0x10_0000), (84, 0x1000), (92, 16),
    ] { put32(&mut bytes, o + offset, value); }
    put16(&mut bytes, o + 40, 4);
    put16(&mut bytes, o + 48, 4);
    put16(&mut bytes, o + 68, 3);
    let s = o + 224;
    bytes[s..s + 8].copy_from_slice(b".text\0\0\0");
    for (offset, value) in [
        (8, 0x300), (12, 0x1000), (16, 0x200), (20, 0x200), (36, 0x6000_0020),
    ] { put32(&mut bytes, s + offset, value); }
    bytes[0x200..0x204].copy_from_slice(&[0x51, 0x72, 0x93, 0xb4]);
    bytes
}

fn limits() -> Limits {
    Limits { max_history: 128, max_text_bytes: 16384, max_caption_bytes: 16384 }
}
fn heap() -> Memory { Memory::new(8192).unwrap() }
fn text() -> CString { CString::new("authored protocol text").unwrap() }
fn endpoints(module: &Module, memory: &Memory) -> (Caller, Presenter) {
    pair(Policy::PrivateAsciiCrLfV1, limits(), module, memory).unwrap()
}
fn refused<T>(result: Result<T, Refusal>, expected: Refusal) {
    match result {
        Err(actual) => assert_eq!(actual, expected),
        Ok(_) => panic!("expected protocol refusal {expected:?}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    active: Option<usize>,
    ui: Option<ThreadId>,
    records: Vec<(Phase, bool, bool, bool, Option<Outcome>, Option<Outcome>)>,
}
fn snapshot(shared: &Shared) -> Snapshot {
    let state = shared.lock().unwrap();
    Snapshot { active: state.active, ui: state.ui_thread,
        records: state.records.iter().map(|r| (r.phase, r.payload.is_some(),
            r.presentation.is_some(), r.close.is_some(), r.proposed, r.ready)).collect() }
}

fn choose(pending: &mut PendingCall<'_, '_>, ui: &mut Presenter, action: Action) -> Completion {
    let mut lease = ui.claim().unwrap().unwrap();
    refused(pending.finish(), Refusal::NotReady);
    let before = snapshot(&ui.shared);
    for button in [Button::Ok, Button::Cancel, Button::Abort, Button::Retry, Button::Ignore,
        Button::Yes, Button::No, Button::TryAgain, Button::Continue]
    {
        if !lease.payload().buttons().contains(&button) {
            refused(ui.decide(&lease, Action::Button(button)), Refusal::ButtonNotOffered);
        }
    }
    assert_eq!(snapshot(&ui.shared), before);
    let intent = ui.decide(&lease, action).unwrap().expect("effective explicit action");
    assert_eq!(pending.poll(), Ok(Progress::AwaitingTeardown));
    refused(pending.finish(), Refusal::NotReady);
    lease.acknowledge_teardown(&intent).unwrap();
    let completed = pending.finish().unwrap();
    refused(pending.finish(), Refusal::Stale);
    completed
}

#[test]
fn messagebox_protocol_payload_survives_cstr_drop_and_retains_actual_borrows() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    let source = CString::new("line one\r\nline two").unwrap();
    let caption = CString::new("caption").unwrap();
    let mut pending = caller.request(&module, &memory, Some(&source), Some(&caption), 0).unwrap();
    drop((source, caption));
    assert!(std::ptr::eq(pending.module(), &module));
    assert!(std::ptr::eq(pending.memory(), &memory));
    assert_eq!(pending.payload().text().original_bytes_with_nul(), Some(&b"line one\r\nline two\0"[..]));
    assert_eq!(pending.payload().text().display(), "line one\r\nline two");
    assert_eq!(pending.payload().caption().display(), "caption");
    refused(ui.claim(), Refusal::UiNotBound);
    ui.bind_ui_thread().unwrap();
    refused(ui.bind_ui_thread(), Refusal::UiAlreadyBound);
    let receipt = choose(&mut pending, &mut ui, Action::Button(Button::Ok));
    assert_eq!(receipt.outcome(), Outcome::UserChoice(Button::Ok));
    drop(pending);
    assert!(caller.issued(&receipt));
    let empty = CString::new("").unwrap();
    let mut pending = caller.request(&module, &memory, Some(&empty), None, 0).unwrap();
    assert_eq!(pending.payload().text().display(), "");
    assert_eq!(pending.payload().text().original_bytes_with_nul(), Some(&b"\0"[..]));
    assert_eq!(pending.payload().caption().original_bytes_with_nul(), None);
    assert_eq!(pending.payload().caption().display(), "Error");
    assert_eq!(choose(&mut pending, &mut ui, Action::Enter).outcome(), Outcome::UserChoice(Button::Ok));
}

#[test]
fn messagebox_protocol_every_button_default_and_escape_is_explicit() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let text = text();
    let cases: [&[Button]; 7] = [
        &[Button::Ok], &[Button::Ok, Button::Cancel],
        &[Button::Abort, Button::Retry, Button::Ignore],
        &[Button::Yes, Button::No, Button::Cancel], &[Button::Yes, Button::No],
        &[Button::Retry, Button::Cancel], &[Button::Cancel, Button::TryAgain, Button::Continue],
    ];
    for (selector, buttons) in cases.iter().enumerate() {
        for (default, expected) in buttons.iter().enumerate() {
            let flags = selector as u32 | ((default as u32) << 8);
            for button in *buttons {
                let mut pending = caller.request(&module, &memory, Some(&text), None, flags).unwrap();
                assert_eq!(pending.payload().buttons(), *buttons);
                assert_eq!(pending.payload().default_button(), *expected);
                assert_eq!(pending.poll(), Ok(Progress::Queued));
                assert_eq!(choose(&mut pending, &mut ui, Action::Button(*button)).outcome(), Outcome::UserChoice(*button));
            }
            let mut pending = caller.request(&module, &memory, Some(&text), None, flags).unwrap();
            assert_eq!(choose(&mut pending, &mut ui, Action::Enter).outcome(), Outcome::UserChoice(*expected));
            drop(pending);
            let mut pending = caller.request(&module, &memory, Some(&text), None, flags).unwrap();
            let mut lease = ui.claim().unwrap().unwrap();
            let escaped = if buttons.contains(&Button::Cancel) { Some(Button::Cancel) }
                else if selector == 0 { Some(Button::Ok) } else { None };
            match escaped {
                Some(button) => {
                    let intent = ui.decide(&lease, Action::Escape).unwrap().unwrap();
                    refused(pending.finish(), Refusal::NotReady);
                    lease.acknowledge_teardown(&intent).unwrap();
                    assert_eq!(pending.finish().unwrap().outcome(), Outcome::UserChoice(button));
                }
                None => {
                    let before = snapshot(&ui.shared);
                    assert!(ui.decide(&lease, Action::Escape).unwrap().is_none());
                    assert_eq!(snapshot(&ui.shared), before);
                    refused(pending.finish(), Refusal::NotReady);
                    let intent = lease.close_without_choice(Abandonment::WindowClose).unwrap();
                    refused(pending.finish(), Refusal::NotReady);
                    lease.acknowledge_teardown(&intent).unwrap();
                    assert_eq!(pending.finish().unwrap().outcome(), Outcome::Abandoned(Abandonment::WindowClose));
                }
            }
        }
    }
    assert_eq!([Button::Ok, Button::Cancel, Button::Abort, Button::Retry, Button::Ignore,
        Button::Yes, Button::No, Button::TryAgain, Button::Continue].map(Button::id),
        [1, 2, 3, 4, 5, 6, 7, 10, 11]);
    // Encoded input 1|4 is selector5, regardless of source-level OR intent.
    let mut pending = caller.request(&module, &memory, Some(&text), None, 1 | 4).unwrap();
    assert_eq!(pending.payload().buttons(), &[Button::Retry, Button::Cancel]);
    assert_eq!(choose(&mut pending, &mut ui, Action::Escape).outcome(), Outcome::UserChoice(Button::Cancel));
}

#[test]
fn messagebox_protocol_refusals_precede_issuance_and_payload_publication() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let twin = fixture.module();
    assert_eq!(module.observation().sha256, twin.observation().sha256);
    assert_ne!(module.origin(), twin.origin());
    let memory = heap();
    let foreign_heap = heap();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    let text = text();
    let before = snapshot(&ui.shared);
    refused(caller.request(&twin, &memory, Some(&text), None, 0), Refusal::ForeignModule);
    refused(caller.request(&module, &foreign_heap, Some(&text), None, 0), Refusal::ForeignMemory);
    refused(caller.request(&module, &memory, None, None, 0), Refusal::NullText);
    for raw in [&b"tab\t"[..], &b"\x1f"[..], &b"\x7f"[..], &b"\x80"[..], &b"\xc3\xa9"[..]] {
        let raw = CString::new(raw).unwrap();
        refused(caller.request(&module, &memory, Some(&raw), None, 0), Refusal::UnsupportedEncoding);
        refused(caller.request(&module, &memory, Some(&text), Some(&raw), 0), Refusal::UnsupportedEncoding);
    }
    for bit in 0..32 {
        if [0, 1, 2, 3, 8, 9].contains(&bit) { continue; }
        refused(caller.request(&module, &memory, Some(&text), None, 1u32 << bit), Refusal::UnsupportedFlags);
    }
    for selector in 7..=15 {
        refused(caller.request(&module, &memory, Some(&text), None, selector), Refusal::UnsupportedButtons);
    }
    for (selector, count) in [1u32, 2, 3, 3, 2, 2, 3].into_iter().enumerate() {
        for default in count..4 {
            refused(caller.request(&module, &memory, Some(&text), None,
                selector as u32 | (default << 8)), Refusal::AbsentDefault);
        }
    }
    let huge = CString::new(vec![b'x'; 16385]).unwrap();
    refused(caller.request(&module, &memory, Some(&huge), None, 0), Refusal::PayloadLimit);
    refused(caller.request(&module, &memory, Some(&text), Some(&huge), 0), Refusal::PayloadLimit);
    assert_eq!(snapshot(&ui.shared), before);
    ui.bind_ui_thread().unwrap();
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    assert_eq!(pending.number(), 1);
    let lease = ui.claim().unwrap().unwrap();
    let before = snapshot(&ui.shared);
    refused(ui.decide(&lease, Action::Button(Button::Yes)), Refusal::ButtonNotOffered);
    assert_eq!(snapshot(&ui.shared), before);
    refused(pending.finish(), Refusal::NotReady);
}

#[test]
fn messagebox_protocol_foreign_same_number_requests_presentations_and_close_intents_refuse() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let (mut a, mut ui_a) = endpoints(&module, &memory);
    let (mut b, mut ui_b) = endpoints(&module, &memory);
    ui_a.bind_ui_thread().unwrap();
    ui_b.bind_ui_thread().unwrap();
    let text = text();
    let mut pa = a.request(&module, &memory, Some(&text), None, 0).unwrap();
    let mut pb = b.request(&module, &memory, Some(&text), None, 0).unwrap();
    let mut la = ui_a.claim().unwrap().unwrap();
    let mut lb = ui_b.claim().unwrap().unwrap();
    assert_eq!(pa.number(), pb.number());
    assert_eq!(la.number(), lb.number());
    let before_a = snapshot(&ui_a.shared);
    let before_b = snapshot(&ui_b.shared);
    refused(ui_a.decide(&lb, Action::Enter), Refusal::ForeignLease);
    // Internal hostile controls exchange actual foreign allocations while all
    // scalar fields/payloads remain identical. No production constructor exists.
    let swapped_presentation = Presentation { shared: Arc::clone(&la.shared),
        key: Authority { issuer: Arc::clone(&la.key.issuer), request: Arc::clone(&la.key.request), index: la.key.index },
        identity: Arc::clone(&lb.identity), payload: Arc::clone(&la.payload) };
    refused(ui_a.decide(&swapped_presentation, Action::Enter), Refusal::ForeignLease);
    let swapped_request = Presentation { shared: Arc::clone(&la.shared),
        key: Authority { issuer: Arc::clone(&la.key.issuer), request: Arc::clone(&lb.key.request), index: la.key.index },
        identity: Arc::clone(&la.identity), payload: Arc::clone(&la.payload) };
    refused(ui_a.decide(&swapped_request, Action::Enter), Refusal::ForeignLease);
    let swapped_issuer = Presentation { shared: Arc::clone(&la.shared),
        key: Authority { issuer: Arc::clone(&lb.key.issuer), request: Arc::clone(&la.key.request), index: la.key.index },
        identity: Arc::clone(&la.identity), payload: Arc::clone(&la.payload) };
    refused(ui_a.decide(&swapped_issuer, Action::Enter), Refusal::ForeignLease);
    assert_eq!(snapshot(&ui_a.shared), before_a);
    assert_eq!(snapshot(&ui_b.shared), before_b);
    let ia = ui_a.decide(&la, Action::Enter).unwrap().unwrap();
    let ib = ui_b.decide(&lb, Action::Enter).unwrap().unwrap();
    let before = snapshot(&ui_a.shared);
    refused(la.acknowledge_teardown(&ib), Refusal::ForeignLease);
    let swapped_close = CloseIntent { shared: Arc::clone(&ia.shared),
        key: Authority { issuer: Arc::clone(&ia.key.issuer), request: Arc::clone(&ia.key.request), index: ia.key.index },
        presentation: Arc::clone(&ia.presentation), identity: Arc::clone(&ib.identity) };
    refused(la.acknowledge_teardown(&swapped_close), Refusal::ForeignLease);
    assert_eq!(snapshot(&ui_a.shared), before);
    refused(pa.finish(), Refusal::NotReady);
    la.acknowledge_teardown(&ia).unwrap();
    lb.acknowledge_teardown(&ib).unwrap();
    let ra = pa.finish().unwrap();
    let rb = pb.finish().unwrap();
    drop((pa, pb));
    assert!(a.issued(&ra));
    assert!(!a.issued(&rb));
    assert!(!b.issued(&ra));
}

#[test]
fn messagebox_protocol_decision_then_teardown_exactly_once_and_stale_history() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let text = text();
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    refused(pending.finish(), Refusal::NotReady);
    let mut lease = ui.claim().unwrap().unwrap();
    refused(ui.claim(), Refusal::Stale);
    refused(pending.cancel_before_presentation(), Refusal::Stale);
    // Reordered ack using authentic request/presentation, but no issued close.
    let premature = CloseIntent { shared: Arc::clone(&lease.shared),
        key: Authority { issuer: Arc::clone(&lease.key.issuer), request: Arc::clone(&lease.key.request), index: lease.key.index },
        presentation: Arc::clone(&lease.identity), identity: Arc::new(CloseIdentity) };
    let before = snapshot(&ui.shared);
    refused(lease.acknowledge_teardown(&premature), Refusal::Stale);
    assert_eq!(snapshot(&ui.shared), before);
    let intent = ui.decide(&lease, Action::Enter).unwrap().unwrap();
    let before = snapshot(&ui.shared);
    refused(ui.decide(&lease, Action::Enter), Refusal::Stale);
    refused(lease.acknowledge_teardown(&premature), Refusal::ForeignLease);
    refused(pending.finish(), Refusal::NotReady);
    assert_eq!(snapshot(&ui.shared), before);
    lease.acknowledge_teardown(&intent).unwrap();
    let ready = snapshot(&ui.shared);
    refused(lease.acknowledge_teardown(&intent), Refusal::Stale);
    assert_eq!(snapshot(&ui.shared), ready);
    assert_eq!(pending.finish().unwrap().outcome(), Outcome::UserChoice(Button::Ok));
    refused(pending.finish(), Refusal::Stale);
    drop(pending);
    let mut next = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    assert_eq!(next.number(), 2);
    let before = snapshot(&ui.shared);
    refused(lease.acknowledge_teardown(&intent), Refusal::ForeignLease);
    refused(ui.decide(&lease, Action::Enter), Refusal::ForeignLease);
    assert_eq!(snapshot(&ui.shared), before);
    assert_eq!(choose(&mut next, &mut ui, Action::Enter).outcome(), Outcome::UserChoice(Button::Ok));
}

#[test]
fn messagebox_protocol_pre_present_disconnect_and_cancellation_are_typed() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let text = text();
    let (mut caller, ui) = endpoints(&module, &memory);
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    drop(ui);
    assert_eq!(pending.poll(), Ok(Progress::Ready));
    assert_eq!(pending.finish().unwrap().outcome(), Outcome::PresenterUnavailable);
    drop(pending);
    refused(caller.request(&module, &memory, Some(&text), None, 0), Refusal::PresenterGone);
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    pending.cancel_before_presentation().unwrap();
    refused(ui.claim(), Refusal::Stale);
    assert_eq!(pending.finish().unwrap().outcome(), Outcome::Abandoned(Abandonment::Programmatic));
    drop(pending);
    let pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let weak = Arc::downgrade(&pending.payload);
    drop(pending);
    assert!(weak.upgrade().is_none());
    assert!(ui.claim().unwrap().is_none());
    let mut next = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    assert_eq!(next.number(), 3);
    assert_eq!(choose(&mut next, &mut ui, Action::Enter).outcome(), Outcome::UserChoice(Button::Ok));
}

#[test]
fn messagebox_protocol_post_present_loss_requires_ack_and_cannot_resume_abandoned_caller() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let text = text();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let mut lease = ui.claim().unwrap().unwrap();
    let weak = Arc::downgrade(&pending.payload);
    drop(pending);
    assert!(weak.upgrade().is_some());
    refused(ui.decide(&lease, Action::Enter), Refusal::CallerGone);
    refused(caller.request(&module, &memory, Some(&text), None, 0), Refusal::Outstanding);
    let intent = lease.close_without_choice(Abandonment::OwnerLost).unwrap();
    refused(caller.request(&module, &memory, Some(&text), None, 0), Refusal::Outstanding);
    lease.acknowledge_teardown(&intent).unwrap();
    // No Completion is created for the dead continuation. A new call is distinct.
    assert_eq!(snapshot(&ui.shared).active, None);
    drop((lease, intent));
    assert!(weak.upgrade().is_none());
    let pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let mut lease = ui.claim().unwrap().unwrap();
    let intent = ui.decide(&lease, Action::Enter).unwrap().unwrap();
    drop(pending);
    assert_eq!(snapshot(&ui.shared).records[1].0, Phase::Closing);
    lease.acknowledge_teardown(&intent).unwrap();
    assert_eq!(snapshot(&ui.shared).records[1].0, Phase::Retired);
    assert_eq!(snapshot(&ui.shared).records[1].5, None);
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let mut lease = ui.claim().unwrap().unwrap();
    // Presenter loss is safe on a foreign thread and does not remove the UI.
    thread::spawn(move || drop(ui)).join().unwrap();
    assert_eq!(pending.poll(), Ok(Progress::Presented));
    refused(pending.finish(), Refusal::NotReady);
    let intent = lease.close_without_choice(Abandonment::ConsumerUnavailable).unwrap();
    refused(pending.finish(), Refusal::NotReady);
    lease.acknowledge_teardown(&intent).unwrap();
    assert_eq!(pending.finish().unwrap().outcome(), Outcome::Abandoned(Abandonment::ConsumerUnavailable));
}

#[test]
fn messagebox_protocol_dropped_presentations_and_intents_never_imply_removal() {
    // Keep the actual Presenter alive: PresenterGone must not mask a released slot.
    for loss in 0..3 {
        let fixture = Fixture::new();
        let module = fixture.module();
        let memory = heap();
        let text = text();
        let (mut caller, mut ui) = endpoints(&module, &memory);
        ui.bind_ui_thread().unwrap();
        let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
        let shared = Arc::downgrade(&ui.shared);
        let payload = Arc::downgrade(&pending.payload);
        let mut lease = Some(ui.claim().unwrap().unwrap());
        let mut intent = if loss == 0 { None } else {
            ui.decide(lease.as_ref().unwrap(), Action::Enter).unwrap()
        };
        match loss {
            0 | 2 => drop(lease.take()), // Presented, or Closing with intent retained.
            1 => drop(intent.take()),   // Closing with Presentation retained.
            _ => unreachable!(),
        }
        assert_eq!(pending.poll(), Ok(if loss == 0 { Progress::Presented } else { Progress::AwaitingTeardown }));
        refused(pending.finish(), Refusal::NotReady);
        drop(pending);
        let before = snapshot(&ui.shared);
        assert_eq!(before.active, Some(0));
        assert_eq!(before.records.len(), 1);
        assert_eq!(before.records[0].5, None);
        refused(caller.request(&module, &memory, Some(&text), None, 0), Refusal::Outstanding);
        assert_eq!(snapshot(&ui.shared), before);
        assert!(payload.upgrade().is_some());
        drop((intent, lease, caller));
        assert!(payload.upgrade().is_some()); // Live UI endpoint retains unresolved state.
        drop(ui);
        assert!(shared.upgrade().is_none());
        assert!(payload.upgrade().is_none());
    }
}

#[test]
fn messagebox_protocol_actual_caller_endpoint_loss_before_and_after_presentation() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let text = text();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let payload = Arc::downgrade(&pending.payload);
    drop(pending);
    drop(caller);
    assert!(ui.claim().unwrap().is_none());
    assert!(payload.upgrade().is_none());
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let payload = Arc::downgrade(&pending.payload);
    let mut lease = ui.claim().unwrap().unwrap();
    drop(pending);
    drop(caller);
    assert!(payload.upgrade().is_some());
    refused(ui.decide(&lease, Action::Enter), Refusal::CallerGone);
    assert_eq!(snapshot(&ui.shared).records[0].0, Phase::Presented);
    let intent = lease.close_without_choice(Abandonment::Programmatic).unwrap();
    assert_eq!(snapshot(&ui.shared).records[0].0, Phase::Closing);
    lease.acknowledge_teardown(&intent).unwrap();
    let retired = snapshot(&ui.shared);
    assert_eq!(retired.records[0].0, Phase::Retired);
    assert_eq!(retired.records[0].5, None);
    assert!(ui.claim().unwrap().is_none());
    drop((intent, lease));
    assert!(payload.upgrade().is_none());
}

#[test]
fn messagebox_protocol_real_threads_bind_once_and_refuse_foreign_operations() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let text = text();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    thread::scope(|scope| {
        scope.spawn(|| refused(caller.request(&module, &memory, Some(&text), None, 0), Refusal::WrongThread))
            .join().unwrap();
    });
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    thread::scope(|scope| {
        scope.spawn(|| {
            refused(pending.poll(), Refusal::WrongThread);
            refused(pending.finish(), Refusal::WrongThread);
            refused(pending.cancel_before_presentation(), Refusal::WrongThread);
        }).join().unwrap();
    });
    // A distinct actual UI thread may be selected once, without numeric proof.
    let (mut ui, mut lease, intent) = thread::spawn(move || {
        ui.bind_ui_thread().unwrap();
        let lease = ui.claim().unwrap().unwrap();
        let intent = ui.decide(&lease, Action::Enter).unwrap().unwrap();
        (ui, lease, intent)
    }).join().unwrap();
    let before = snapshot(&ui.shared);
    refused(ui.claim(), Refusal::WrongThread);
    refused(ui.decide(&lease, Action::Enter), Refusal::WrongThread);
    refused(lease.close_without_choice(Abandonment::Programmatic), Refusal::WrongThread);
    refused(lease.acknowledge_teardown(&intent), Refusal::WrongThread);
    assert_eq!(snapshot(&ui.shared), before);
    refused(pending.finish(), Refusal::NotReady);
    // The now-absent UI thread cannot be replaced; drops stay Rust-only.
    drop((intent, lease, ui, pending));
}

#[test]
fn messagebox_protocol_actual_distinct_ui_thread_can_complete_and_foreign_drop_is_safe() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let text = text();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    thread::spawn(move || {
        ui.bind_ui_thread().unwrap();
        let mut lease = ui.claim().unwrap().unwrap();
        let intent = ui.decide(&lease, Action::Enter).unwrap().unwrap();
        lease.acknowledge_teardown(&intent).unwrap();
    }).join().unwrap();
    assert_eq!(pending.finish().unwrap().outcome(), Outcome::UserChoice(Button::Ok));
    drop(pending);
    // A different live channel, with PendingCall destroyed off its caller thread.
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    thread::scope(|scope| { scope.spawn(move || drop(pending)).join().unwrap(); });
    assert!(ui.claim().unwrap().is_none());
    let mut next = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    assert_eq!(next.number(), 2);
    assert_eq!(choose(&mut next, &mut ui, Action::Enter).outcome(), Outcome::UserChoice(Button::Ok));
}

#[test]
fn messagebox_protocol_busy_and_poison_refuse_and_destruction_never_acks() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let text = text();
    let (mut caller, ui) = endpoints(&module, &memory);
    let shared = Arc::clone(&ui.shared);
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let guard = shared.lock().unwrap();
    refused(pending.poll(), Refusal::Busy);
    drop(ui); // Contended nonblocking Drop defers cleanup; it does not wait.
    refused(pending.finish(), Refusal::Busy);
    drop(guard);
    assert_eq!(pending.finish().unwrap().outcome(), Outcome::PresenterUnavailable);
    drop(pending);
    drop(shared);
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let mut lease = ui.claim().unwrap().unwrap();
    let shared = Arc::clone(&ui.shared);
    assert!(thread::spawn(move || {
        let _guard = shared.state.lock().unwrap();
        panic!("authored intentional mutex poison; protocol controls only");
    }).join().is_err());
    refused(pending.poll(), Refusal::Poisoned);
    refused(pending.finish(), Refusal::Poisoned);
    refused(ui.decide(&lease, Action::Enter), Refusal::Poisoned);
    refused(lease.close_without_choice(Abandonment::OwnerLost), Refusal::Poisoned);
    let weak = Arc::downgrade(&pending.payload);
    let shared = Arc::downgrade(&ui.shared);
    drop((pending, lease, ui)); // Only destruction can recover poisoned storage.
    assert!(weak.upgrade().is_some()); // Presented state retained by caller.
    drop(caller);
    assert!(shared.upgrade().is_none());
    assert!(weak.upgrade().is_none());
}

#[test]
fn messagebox_protocol_budgets_never_recycle_and_boundary_strings_are_owned() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    refused(pair(Policy::Disabled, limits(), &module, &memory), Refusal::Disabled);
    for bound in [0, 129, usize::MAX] {
        refused(pair(Policy::PrivateAsciiCrLfV1, Limits { max_history: bound, ..limits() }, &module, &memory), Refusal::InvalidLimits);
    }
    for bound in [0, 16385, usize::MAX] {
        refused(pair(Policy::PrivateAsciiCrLfV1, Limits { max_text_bytes: bound, ..limits() }, &module, &memory), Refusal::InvalidLimits);
        refused(pair(Policy::PrivateAsciiCrLfV1, Limits { max_caption_bytes: bound, ..limits() }, &module, &memory), Refusal::InvalidLimits);
    }
    let (mut caller, mut ui) = pair(Policy::PrivateAsciiCrLfV1,
        Limits { max_history: 1, ..limits() }, &module, &memory).unwrap();
    ui.bind_ui_thread().unwrap();
    let full = CString::new(vec![b'~'; 16384]).unwrap();
    let mut pending = caller.request(&module, &memory, Some(&full), Some(&full), 0).unwrap();
    assert_eq!(pending.payload().text().original_bytes_with_nul().unwrap().len(), 16385);
    assert_eq!(pending.payload().caption().display().len(), 16384);
    drop(full);
    choose(&mut pending, &mut ui, Action::Enter);
    drop(pending);
    let before = snapshot(&ui.shared);
    let text = text();
    refused(caller.request(&module, &memory, Some(&text), None, 0), Refusal::HistoryExhausted);
    assert_eq!(snapshot(&ui.shared), before);
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    for expected in 1..=128 {
        let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
        assert_eq!(pending.number(), expected);
        pending.cancel_before_presentation().unwrap();
        assert_eq!(pending.finish().unwrap().outcome(), Outcome::Abandoned(Abandonment::Programmatic));
    }
    refused(caller.request(&module, &memory, Some(&text), None, 0), Refusal::HistoryExhausted);
    assert_eq!(snapshot(&ui.shared).records.len(), 128);
    let (mut caller, _ui) = pair(Policy::PrivateAsciiCrLfV1,
        Limits { max_history: 1, max_text_bytes: 1, max_caption_bytes: 1 }, &module, &memory).unwrap();
    let one = CString::new("x").unwrap();
    refused(caller.request(&module, &memory, Some(&one), None, 0), Refusal::PayloadLimit);
    let mut pending = caller.request(&module, &memory, Some(&one), Some(&one), 0).unwrap();
    pending.cancel_before_presentation().unwrap();
    pending.finish().unwrap();
}

#[test]
fn messagebox_protocol_real_weak_witnesses_release_payload_and_authority_at_actual_last_owner() {
    let fixture = Fixture::new();
    let module = fixture.module();
    let memory = heap();
    let text = text();
    let (mut caller, mut ui) = endpoints(&module, &memory);
    ui.bind_ui_thread().unwrap();
    let shared = Arc::downgrade(&ui.shared);
    let issuer = Arc::downgrade(&ui.shared.issuer);
    let mut pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
    let payload = Arc::downgrade(&pending.payload);
    let request = Arc::downgrade(&pending.key.request);
    let mut lease = ui.claim().unwrap().unwrap();
    let presentation = Arc::downgrade(&lease.identity);
    let intent = ui.decide(&lease, Action::Enter).unwrap().unwrap();
    let close = Arc::downgrade(&intent.identity);
    lease.acknowledge_teardown(&intent).unwrap();
    let completion = pending.finish().unwrap();
    drop(pending);
    assert!(payload.upgrade().is_some()); // Actual UI consumer still owns text.
    drop(lease);
    assert!(payload.upgrade().is_none());
    assert!(presentation.upgrade().is_some()); // Intent retains actual lease ID.
    drop(intent);
    assert!(presentation.upgrade().is_none());
    assert!(close.upgrade().is_none());
    drop((caller, ui));
    assert!(shared.upgrade().is_none());
    assert!(issuer.upgrade().is_some());
    assert!(request.upgrade().is_some()); // Authentic historical receipt only.
    drop(completion);
    assert!(issuer.upgrade().is_none());
    assert!(request.upgrade().is_none());
}

#[test]
fn messagebox_protocol_concurrent_pending_loss_while_ui_holds_each_phase_defers_cleanup() {
    use std::sync::mpsc;
    use std::time::Duration;
    // Deterministic overlap: actual UI thread owns the actual state Mutex while
    // the caller destroys PendingCall. The reply channel proves Drop did not
    // wait for that lock. This is a bounded schedule, not physical teardown proof.
    for phase in [Phase::Queued, Phase::Presented, Phase::Closing] {
        let fixture = Fixture::new();
        let module = fixture.module();
        let memory = heap();
        let text = text();
        let (mut caller, mut ui) = endpoints(&module, &memory);
        let pending = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
        let payload = Arc::downgrade(&pending.payload);
        let shared = Arc::clone(&ui.shared);
        let shared_weak = Arc::downgrade(&shared);
        let (held_tx, held_rx) = mpsc::sync_channel(0);
        let (dropped_tx, dropped_rx) = mpsc::sync_channel(0);
        let ui_thread = thread::spawn(move || {
            ui.bind_ui_thread().unwrap();
            let mut lease = if phase == Phase::Queued { None } else { ui.claim().unwrap() };
            let mut intent = if phase == Phase::Closing {
                ui.decide(lease.as_ref().unwrap(), Action::Enter).unwrap()
            } else { None };
            let state = Arc::clone(&ui.shared);
            let guard = state.state.lock().unwrap();
            assert_eq!(guard.records[0].phase, phase);
            held_tx.send(()).unwrap();
            dropped_rx.recv_timeout(Duration::from_secs(5)).expect("PendingCall Drop blocked on UI state lock");
            assert!(!guard.records[0].request.caller_alive.load(Ordering::Acquire));
            assert_eq!(guard.active, Some(0));
            assert_eq!(guard.records[0].phase, phase);
            assert!(guard.records[0].payload.is_some());
            assert!(guard.records[0].ready.is_none());
            drop(guard);
            if phase == Phase::Queued {
                assert!(ui.claim().unwrap().is_none());
            } else {
                let lease = lease.as_mut().unwrap();
                if phase == Phase::Presented {
                    refused(ui.decide(lease, Action::Enter), Refusal::CallerGone);
                    intent = Some(lease.close_without_choice(Abandonment::OwnerLost).unwrap());
                }
                let before = snapshot(&ui.shared);
                assert_eq!(before.active, Some(0));
                assert_eq!(before.records[0].0, Phase::Closing);
                assert_eq!(before.records[0].5, None);
                lease.acknowledge_teardown(intent.as_ref().unwrap()).unwrap();
            }
            let after = snapshot(&ui.shared);
            assert_eq!(after.active, None);
            assert_eq!(after.records[0].0, Phase::Retired);
            assert_eq!(after.records[0].5, None); // No completion for the destroyed continuation.
            drop((intent, lease));
            ui
        });
        held_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(pending); // Runs concurrently with the bound UI thread holding State.
        dropped_tx.send(()).unwrap();
        let ui = ui_thread.join().unwrap();
        assert!(payload.upgrade().is_none());
        let mut next = caller.request(&module, &memory, Some(&text), None, 0).unwrap();
        assert_eq!(next.number(), 2);
        next.cancel_before_presentation().unwrap();
        assert_eq!(next.finish().unwrap().outcome(), Outcome::Abandoned(Abandonment::Programmatic));
        drop(next);
        drop((caller, ui, shared));
        assert!(shared_weak.upgrade().is_none());
    }
}
