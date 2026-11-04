#![cfg(feature = "private-fixtures")]

//! Supplied-provider differential for unchanged original53a01f/4cfcdb.
//! Does not invoke a graphics pump or reconstruct/import a Windows widget tree.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_vm::family_label::*;
use sims_vm::{Program, Snapshot, Vm, World, family::Metadata, person::WorldState};
use std::collections::BTreeMap;

const VIEW: u32 = 0x10000000;
const CP: u32 = VIEW + 0x1000;
const CP2: u32 = VIEW + 0x2000;
const WIDGET: u32 = VIEW + 0x3000;
const WIDGET2: u32 = VIEW + 0x4000;
const FAMILY: u32 = VIEW + 0x9000;
const FAMILY2: u32 = VIEW + 0xa000;
const PERSON: u32 = VIEW + 0xb000;

fn unhex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn cases() -> Value {
    serde_json::from_str(include_str!("fixtures/family-label-supplied.json")).unwrap()
}

/// Raw byte owners below are explicit test-provider fixtures, not new VM World
/// fields or a claim that bool metadata recovers unknown native padding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Host {
    case: Value,
    vm: Vm,
    fields: Fields,
    application_family: Option<u32>,
    cp: BTreeMap<u32, Vec<u8>>,
    pointers: BTreeMap<u32, u32>,
    buffers: BTreeMap<u32, Vec<u8>>,
    widgets: BTreeMap<u32, Vec<u8>>,
    raw_dirty: BTreeMap<u32, u8>,
    ledger: Vec<Value>,
}
impl Host {
    fn new(case: &Value) -> Self {
        let mut people = WorldState {
            neighbor_slots: Some(vec![]),
            ..Default::default()
        };
        for (id, cached, dirty) in [
            (
                1,
                case["cached"].as_i64().unwrap() as i32,
                case["dirty"].as_u64().unwrap() != 0,
            ),
            (2, 17, false),
        ] {
            people.families.insert(id, vec![]);
            people.family_metadata.insert(
                id,
                Metadata {
                    flags: 0,
                    dirty,
                    friend_count: Some(cached),
                    member_guids: None,
                },
            );
        }
        let mut vm = Vm::new(
            World {
                person: Some(people),
                globals: [(19, 2)].into(),
                ..Default::default()
            },
            0x12345678,
        );
        vm.tick = 88444;
        vm.lot_rng_state = Some(0xabcdef01);
        vm.crt_rng_state = Some(0x76543210);
        Self {
            case: case.clone(),
            vm,
            fields: Fields {
                owner: VIEW,
                state: Some(CP),
                label: Some(WIDGET),
            },
            application_family: (case["app_present"] == true && case["family_present"] == true)
                .then_some(FAMILY),
            cp: [
                (CP, b"not-published".to_vec()),
                (CP2, b"not-published".to_vec()),
            ]
            .into(),
            pointers: [(CP, CP + 0x300), (CP2, CP2 + 0x300)].into(),
            buffers: [
                (CP + 0x300, b"not-published".to_vec()),
                (CP2 + 0x300, b"not-published".to_vec()),
            ]
            .into(),
            widgets: [
                (
                    WIDGET,
                    case["label_text"].as_str().unwrap().as_bytes().to_vec(),
                ),
                (WIDGET2, b"second-widget".to_vec()),
            ]
            .into(),
            raw_dirty: [
                (FAMILY, case["dirty"].as_u64().unwrap() as u8),
                (FAMILY2, 0),
            ]
            .into(),
            ledger: vec![],
        }
    }
    fn unavailable(s: &str) -> Failure {
        Failure::MissingInput(s.into())
    }
    fn service(&mut self, request: &Request, state: u32) -> Result<Reply, Failure> {
        use NumericReply as P;
        use NumericRequest as R;
        Ok(match request {
            Request::Numeric(r) => Reply::Numeric(match r {
                R::LotQuery => {
                    self.ledger.push(json!(["count", state]));
                    self.ledger.push(json!(["lot", self.case["app_route"]]));
                    P::Boolean(self.case["app_route"] == true)
                }
                R::ApplicationFamily => P::Owner(self.application_family),
                R::SelectedPerson => {
                    let n = self.case["selected"].as_i64().unwrap() as u32;
                    self.ledger.push(json!(["selected", n as i32]));
                    P::Number(n)
                }
                R::Person { index } => {
                    self.ledger.push(json!(["person", *index as i32]));
                    P::Owner((self.case["person_present"] == true).then_some(PERSON))
                }
                R::NeighborWord { person } => {
                    if *person != PERSON {
                        return Err(Self::unavailable("person word owner absent"));
                    }
                    P::Word(self.case["neighbor"].as_i64().unwrap() as u16)
                }
                R::Family { neighbor } => {
                    self.ledger.push(json!(["family", *neighbor]));
                    P::Owner((self.case["family_present"] == true).then_some(FAMILY))
                }
                R::Friends { family } => {
                    self.ledger.push(json!(["friends", *family]));
                    let id = match *family {
                        FAMILY => 1,
                        FAMILY2 => 2,
                        _ => return Err(Self::unavailable("family cache owner absent")),
                    };
                    let people = self
                        .vm
                        .world
                        .person
                        .as_mut()
                        .ok_or_else(|| Self::unavailable("neighborhood absent"))?;
                    // Existing private execution method: inspect does NOT clear
                    // dirty state and is intentionally not substituted here.
                    let value = people
                        .resolve_family_friend_count(id, self.vm.world.autonomy_constants.as_ref())
                        .map_err(Failure::MissingInput)?;
                    if !people.family_metadata[&id].dirty {
                        self.raw_dirty.insert(*family, 0);
                    }
                    P::Count(value)
                }
                R::Format { value } => {
                    let bytes = value.to_string().into_bytes();
                    self.ledger.push(json!(["format", bytes]));
                    P::Bytes(bytes)
                }
                R::Store { state, bytes } => {
                    let output = self
                        .cp
                        .get_mut(state)
                        .ok_or_else(|| Self::unavailable("CPState text owner absent"))?;
                    *output = bytes.clone();
                    let pointer = self
                        .pointers
                        .get(state)
                        .ok_or_else(|| Self::unavailable("CPState CString pointer unavailable"))?;
                    *self.buffers.get_mut(pointer).ok_or_else(|| {
                        Self::unavailable("CPState CString backing unavailable")
                    })? = bytes.clone();
                    self.ledger.push(json!(["store", *state, bytes]));
                    P::Done
                }
                R::CString { state } => {
                    let buffer = *self
                        .pointers
                        .get(state)
                        .ok_or_else(|| Self::unavailable("CPState CString pointer unavailable"))?;
                    P::Text(CountText {
                        state: *state,
                        buffer,
                    })
                }
            }),
            Request::Window(r) => Reply::Window(match r {
                WindowRequest::Count { .. } => panic!("combined Call must own numeric child"),
                WindowRequest::Temporary { source } => {
                    let bytes = self
                        .buffers
                        .get(&source.buffer)
                        .ok_or_else(|| Self::unavailable("CString source owner absent"))?
                        .clone();
                    self.ledger.push(json!(["temporary", bytes]));
                    WindowReply::Bytes(bytes)
                }
                WindowRequest::Equal { left, widget } => {
                    let actual = self
                        .widgets
                        .get(widget)
                        .ok_or_else(|| Self::unavailable("label comparison owner absent"))?;
                    let equal = left == actual;
                    self.ledger.push(json!(["compare", equal]));
                    if let Some(mutation) = self.case["mutation"].as_str() {
                        self.ledger.push(json!(["mutation", mutation]));
                        match mutation {
                            "cpstate" => self.fields.state = Some(CP2),
                            "widget" => self.fields.label = Some(WIDGET2),
                            "family" => self.application_family = Some(FAMILY2),
                            "dirty-again" => {
                                let cache = self
                                    .vm
                                    .world
                                    .person
                                    .as_mut()
                                    .unwrap()
                                    .family_metadata
                                    .get_mut(&1)
                                    .unwrap();
                                cache.friend_count = Some(1234);
                                cache.dirty = true;
                                self.raw_dirty.insert(FAMILY, 255);
                            }
                            "missing-neighborhood" | "dirty-missing-neighborhood" => {
                                self.vm.world.person.as_mut().unwrap().neighbor_slots = None;
                                if mutation == "dirty-missing-neighborhood" {
                                    self.vm
                                        .world
                                        .person
                                        .as_mut()
                                        .unwrap()
                                        .family_metadata
                                        .get_mut(&1)
                                        .unwrap()
                                        .dirty = true;
                                    self.raw_dirty.insert(FAMILY, 255);
                                }
                            }
                            _ => panic!("unsupported fixture mutation"),
                        }
                    }
                    WindowReply::Equal(equal)
                }
                WindowRequest::SetText { widget, bytes } => {
                    *self
                        .widgets
                        .get_mut(widget)
                        .ok_or_else(|| Self::unavailable("label SetText owner absent"))? =
                        bytes.clone();
                    self.ledger.push(json!(["set", *widget, bytes]));
                    WindowReply::Done
                }
                WindowRequest::Destroy { temporary } => {
                    self.ledger.push(json!(["destroy", *temporary]));
                    WindowReply::Done
                }
            }),
        })
    }
    fn assert_vm_effects(&self, original: &Value, before: &Vm) {
        let mut expected = before.clone();
        let people = expected.world.person.as_mut().unwrap();
        for (id, name) in [(1, "family"), (2, "family2")] {
            let cache = people.family_metadata.get_mut(&id).unwrap();
            cache.friend_count = Some(original[format!("{name}_cached")].as_i64().unwrap() as i32);
            cache.dirty = original[format!("{name}_dirty")].as_u64().unwrap() != 0;
        }
        if matches!(
            self.case["mutation"].as_str(),
            Some("missing-neighborhood" | "dirty-missing-neighborhood")
        ) {
            people.neighbor_slots = None;
        }
        assert_eq!(
            self.vm, expected,
            "whole VM must differ only by supplied owner mutation and executed cache publication"
        );
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Runner {
    Window(Call),
    Numeric(Numeric),
}
impl Runner {
    fn advance(&mut self, fields: Option<&Fields>) -> Result<Option<Request>, Error> {
        match self {
            Self::Window(c) => c.advance(fields),
            Self::Numeric(c) => c.advance().map(|r| r.map(Request::Numeric)),
        }
    }
    fn state(&self) -> u32 {
        match self {
            Self::Window(c) => c.numeric().map_or(0, Numeric::state),
            Self::Numeric(c) => c.state(),
        }
    }
    fn ack(&mut self, r: &Request, p: Reply) -> Result<(), Error> {
        match self {
            Self::Window(c) => c.acknowledge(r, p),
            Self::Numeric(c) => match (r, p) {
                (Request::Numeric(r), Reply::Numeric(p)) => c.acknowledge(r, p),
                _ => Err(Error::ReplyMismatch),
            },
        }
    }
    fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Window(c) => c.validate(),
            Self::Numeric(c) => c.validate(),
        }
    }
    fn complete(&self) -> bool {
        match self {
            Self::Window(c) => c.complete(),
            Self::Numeric(c) => c.complete(),
        }
    }
    fn reject(&self, r: &Request, e: Failure) -> Error {
        match self {
            Self::Window(c) => c.reject(r, e).unwrap(),
            Self::Numeric(_) => Error::Unsupported {
                request: r.clone(),
                failure: e,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Pair {
    runner: Runner,
    host: Host,
}
fn cold(pair: &mut Pair) {
    // The UI continuation/host and the independent VM checkpoint both survive.
    let p = Program::default();
    let snapshot = Snapshot::capture(&p, &pair.host.vm);
    let restored: Snapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert_eq!(pair.host.vm, restored.restore(&p).unwrap());
    let bytes = serde_json::to_vec(pair).unwrap();
    *pair = serde_json::from_slice(&bytes).unwrap();
    pair.runner.validate().unwrap();
    assert_eq!(serde_json::to_vec(pair).unwrap(), bytes);
}
fn run(case: &Value, with_cold: bool) -> (Pair, Option<Error>, usize) {
    let host = Host::new(case);
    let runner = if case["entry"].as_u64() == Some(0x4cfcdb) {
        Runner::Numeric(Numeric::new(CP))
    } else {
        Runner::Window(Call::new(VIEW, case["enabled"].as_u64().unwrap() as u32))
    };
    let mut pair = Pair { runner, host };
    let mut cuts = 0;
    for _ in 0..256 {
        if with_cold {
            cold(&mut pair);
            cuts += 1;
        }
        let Some(request) = pair.runner.advance(Some(&pair.host.fields)).unwrap() else {
            assert!(pair.runner.complete());
            if let Runner::Window(c) = &pair.runner {
                assert_eq!(c.result(), Some(0));
            }
            if with_cold {
                cold(&mut pair);
                cuts += 1;
            }
            return (pair, None, cuts);
        };
        let pending = pair.runner.clone();
        assert_eq!(pair.runner.advance(None).unwrap(), Some(request.clone()));
        assert_eq!(
            pair.runner, pending,
            "repeated pending advance must not reread fields or re-execute children"
        );
        if with_cold {
            cold(&mut pair);
            cuts += 1;
        }
        match pair.host.service(&request, pair.runner.state()) {
            Ok(reply) => pair.runner.ack(&request, reply).unwrap(),
            Err(failure) => {
                let error = pair.runner.reject(&request, failure);
                assert_eq!(
                    pair.runner, pending,
                    "unsupported child retains pending ownership"
                );
                if with_cold {
                    cold(&mut pair);
                    cuts += 1;
                }
                return (pair, Some(error), cuts);
            }
        }
        if with_cold {
            cold(&mut pair);
            cuts += 1;
        }
    }
    panic!("bounded semantic callback did not return")
}

fn original_ledger(original: &Value) -> Vec<Value> {
    let mut result = vec![];
    let mut temporaries = vec![];
    for e in original["events"].as_array().unwrap() {
        let entry = match e["event"].as_str().unwrap() {
            "native-entry" => match e["pc"].as_u64().unwrap() {
                0x4cfcdb => Some(json!(["count", e["ecx"]])),
                0x410260 => Some(json!(["friends", e["ecx"]])),
                _ => None,
            },
            "supplied-lot-query" => Some(json!(["lot", e["value"]])),
            "supplied-selected-person" => Some(json!(["selected", e["value"]])),
            "supplied-person-lookup" => Some(json!(["person", e["index"]])),
            "supplied-family-lookup" => Some(json!(["family", e["neighbor"]])),
            "supplied-decimal-format" => {
                Some(json!(["format", unhex(e["bytes"].as_str().unwrap())]))
            }
            "supplied-cpstate-string-copy" => Some(json!([
                "store",
                e["owner"].as_u64().unwrap() as u32 - 0x234,
                unhex(e["bytes"].as_str().unwrap())
            ])),
            "supplied-temporary-string" => {
                temporaries.push(e["owner"].as_u64().unwrap());
                Some(json!(["temporary", unhex(e["bytes"].as_str().unwrap())]))
            }
            "native-compare-return" => Some(json!(["compare", e["low_al"] == 1])),
            "supplied-after-compare-mutation" => Some(json!(["mutation", e["kind"]])),
            "supplied-widget-set-text" => Some(json!([
                "set",
                e["widget"],
                unhex(e["bytes"].as_str().unwrap())
            ])),
            "supplied-string-destroy" => Some(json!([
                "destroy",
                temporaries
                    .iter()
                    .position(|p| Some(*p) == e["owner"].as_u64())
                    .unwrap()
                    + 1
            ])),
            _ => None,
        };
        if let Some(entry) = entry {
            result.push(entry);
        }
    }
    result
}

#[test]
fn all_37_original_vectors_match_with_cold_cuts_and_exact_vm_ownership() {
    let all = cases();
    assert_eq!(all["case_count"], 37);
    let mut cuts = 0;
    for row in all["cases"].as_array().unwrap() {
        let case = &row["input"];
        let original = &row["original"];
        let before = Host::new(case).vm;
        let (warm, warm_error, _) = run(case, false);
        let (restored, cold_error, count) = run(case, true);
        cuts += count;
        assert_eq!(
            warm, restored,
            "full host/VM/continuation differs: {}",
            case["name"]
        );
        assert_eq!(warm_error, cold_error);
        assert_eq!(
            warm.host.ledger,
            original_ledger(original),
            "protocol differs: {}",
            case["name"]
        );
        assert_eq!(
            warm.host.raw_dirty[&FAMILY],
            original["family_dirty"].as_u64().unwrap() as u8
        );
        assert_eq!(
            warm.host.raw_dirty[&FAMILY2],
            original["family2_dirty"].as_u64().unwrap() as u8
        );
        for (id, name) in [(CP, "cp_text"), (CP2, "cp2_text")] {
            assert_eq!(
                &warm.host.cp[&id],
                original[name].as_str().unwrap().as_bytes()
            );
        }
        for (id, name) in [(WIDGET, "widget_text"), (WIDGET2, "widget2_text")] {
            assert_eq!(
                &warm.host.widgets[&id],
                original[name].as_str().unwrap().as_bytes()
            );
        }
        warm.host.assert_vm_effects(original, &before);
        if original["error"].is_null() {
            assert!(warm_error.is_none());
        } else {
            // Native supplied invalid pointer faults; the Rust host reports the
            // unavailable prerequisite, not a fabricated original error number.
            assert_eq!(original["error"]["pc"], 0x427be4);
            assert_eq!(
                warm_error,
                Some(Error::Unsupported {
                    request: Request::Numeric(NumericRequest::Friends { family: FAMILY }),
                    failure: Failure::MissingInput("native friend scan slots unavailable".into()),
                })
            );
        }
    }
    println!(
        "original_vectors=37 compact_cold_cuts={cuts}; full VM/RNG unchanged except executed cache and supplied mutations"
    );
    assert!(cuts > 1000);
}

#[test]
fn gate_unknowns_wrong_replies_and_late_fields_are_resumable() {
    for raw in [0, 0x100, 0xffffff00] {
        let mut call = Call::new(VIEW, raw);
        assert_eq!(call.advance(None), Ok(None));
        assert_eq!(call.result(), Some(0));
    }
    let mut call = Call::new(VIEW, 1);
    let before = call.clone();
    assert_eq!(
        call.advance(None),
        Err(Error::FieldsUnavailable { owner: VIEW })
    );
    assert_eq!(call, before);
    let request = call
        .advance(Some(&Fields {
            owner: VIEW,
            state: Some(CP),
            label: Some(WIDGET),
        }))
        .unwrap()
        .unwrap();
    let pending = call.clone();
    assert_eq!(
        call.acknowledge(&request, Reply::Window(WindowReply::Done)),
        Err(Error::ReplyMismatch)
    );
    assert_eq!(call, pending);
    let error = call
        .reject(
            &request,
            Failure::Unimplemented("real lot query owner unavailable".into()),
        )
        .unwrap();
    assert!(matches!(error, Error::Unsupported { .. }));
    assert_eq!(call, pending);
    // The actual query can be supplied later; a rejection is not a reset or
    // a completed fake zero value.
    call.acknowledge(&request, Reply::Numeric(NumericReply::Boolean(true)))
        .unwrap();
    let acknowledged = call.clone();
    assert_eq!(
        call.acknowledge(&request, Reply::Numeric(NumericReply::Boolean(true))),
        Err(Error::ReplyMismatch)
    );
    assert_eq!(call, acknowledged);
    assert_eq!(
        call.advance(None),
        Ok(Some(Request::Numeric(NumericRequest::ApplicationFamily)))
    );
}

#[test]
fn null_cpstate_is_not_rejected_before_executing_family_cache() {
    let all = cases();
    let c = &all["cases"][0]["input"];
    let mut host = Host::new(c);
    host.fields.state = Some(0);
    let mut call = Call::new(VIEW, 1);
    for _ in 0..100 {
        let request = call.advance(Some(&host.fields)).unwrap().unwrap();
        if matches!(
            &request,
            Request::Numeric(NumericRequest::Store { state: 0, .. })
        ) {
            assert!(!host.vm.world.person.as_ref().unwrap().family_metadata[&1].dirty);
            assert_eq!(
                host.vm.world.person.as_ref().unwrap().family_metadata[&1].friend_count,
                Some(0)
            );
            assert_eq!(host.cp[&CP], b"not-published");
            assert!(matches!(
                host.service(&request, call.numeric().unwrap().state()),
                Err(Failure::MissingInput(_))
            ));
            return;
        }
        let reply = host
            .service(&request, call.numeric().map_or(0, Numeric::state))
            .unwrap();
        call.acknowledge(&request, reply).unwrap();
    }
    panic!("late CPState fault boundary not reached")
}

#[test]
fn unknown_family_and_format_failure_never_publish_a_fake_result() {
    let mut number = Numeric::new(CP);
    let r = number.advance().unwrap().unwrap();
    number.acknowledge(&r, NumericReply::Boolean(true)).unwrap();
    let r = number.advance().unwrap().unwrap();
    let before = number.clone();
    assert_eq!(
        number.acknowledge(&r, NumericReply::Owner(Some(0))),
        Err(Error::ReplyMismatch)
    );
    assert_eq!(number, before);
    number.acknowledge(&r, NumericReply::Owner(None)).unwrap();
    let r = number.advance().unwrap().unwrap();
    assert_eq!(r, NumericRequest::Format { value: 0 });
    assert!(number.result().is_none());
    assert_eq!(number.advance(), Ok(Some(r)));
    let mut corrupted = serde_json::to_value(&number).unwrap();
    corrupted["stage"] = json!("Complete");
    let bad: Numeric = serde_json::from_value(corrupted).unwrap();
    assert_eq!(bad.validate(), Err(Error::InvalidContinuation));
}

#[test]
fn unavailable_fields_after_temporary_preserve_reply_and_owned_text() {
    let mut update = Update::new(VIEW, 1);
    let fields = Fields {
        owner: VIEW,
        state: Some(CP),
        label: Some(WIDGET),
    };
    let r = update.advance(Some(&fields)).unwrap().unwrap();
    update
        .acknowledge(
            &r,
            WindowReply::Text(CountText {
                state: CP,
                buffer: CP + 0x300,
            }),
        )
        .unwrap();
    let r = update.advance(None).unwrap().unwrap();
    update
        .acknowledge(&r, WindowReply::Bytes(b"7".to_vec()))
        .unwrap();
    let before = update.clone();
    assert_eq!(
        update.advance(None),
        Err(Error::FieldsUnavailable { owner: VIEW })
    );
    assert_eq!(update, before);
    assert_eq!(
        update.advance(Some(&Fields {
            owner: VIEW,
            state: Some(CP2),
            label: Some(WIDGET2)
        })),
        Ok(Some(WindowRequest::Equal {
            left: b"7".to_vec(),
            widget: WIDGET2
        }))
    );
}

#[test]
fn returned_cstring_buffer_identity_survives_cpstate_pointer_replacement() {
    let all = cases();
    let mut host = Host::new(&all["cases"][0]["input"]);
    let mut number = Numeric::new(CP);
    for _ in 0..32 {
        let Some(request) = number.advance().unwrap() else {
            break;
        };
        let Reply::Numeric(reply) = host
            .service(&Request::Numeric(request.clone()), number.state())
            .unwrap()
        else {
            unreachable!()
        };
        number.acknowledge(&request, reply).unwrap();
    }
    let source = number.result().unwrap();
    assert_eq!(
        source,
        CountText {
            state: CP,
            buffer: CP + 0x300
        }
    );
    // Deliberate supplied-provider mutation after the actual native pointer
    // read. The saved reference must not become a fresh CPState field query.
    host.pointers.insert(CP, CP + 0x500);
    host.buffers
        .insert(CP + 0x500, b"different-buffer".to_vec());
    let host: Host = serde_json::from_slice(&serde_json::to_vec(&host).unwrap()).unwrap();
    let mut restored = host;
    assert_eq!(
        restored.service(&Request::Window(WindowRequest::Temporary { source }), 0),
        Ok(Reply::Window(WindowReply::Bytes(b"0".to_vec())))
    );
}

#[test]
fn unsupported_formatter_preserves_executed_cache_and_pending_ownership() {
    let all = cases();
    let mut host = Host::new(&all["cases"][0]["input"]);
    let mut call = Call::new(VIEW, 1);
    for _ in 0..32 {
        let request = call.advance(Some(&host.fields)).unwrap().unwrap();
        if matches!(
            request,
            Request::Numeric(NumericRequest::Format { value: 0 })
        ) {
            let pending = call.clone();
            let before = host.vm.clone();
            assert!(!before.world.person.as_ref().unwrap().family_metadata[&1].dirty);
            assert_eq!(host.cp[&CP], b"not-published");
            let rejected = call
                .reject(
                    &request,
                    Failure::Unimplemented("decimal formatter unbound".into()),
                )
                .unwrap();
            assert!(matches!(rejected, Error::Unsupported { .. }));
            assert_eq!(call, pending);
            assert_eq!(host.vm, before);
            let mut pair = Pair {
                runner: Runner::Window(call),
                host,
            };
            cold(&mut pair);
            assert_eq!(pair.runner.advance(None), Ok(Some(request)));
            return;
        }
        let reply = host
            .service(&request, call.numeric().map_or(0, Numeric::state))
            .unwrap();
        call.acknowledge(&request, reply).unwrap();
    }
    panic!("format boundary not reached")
}

#[test]
fn absent_family_does_not_require_neighborhood_or_process_tuning() {
    let all = cases();
    let mut host = Host::new(&all["cases"][0]["input"]);
    host.application_family = None;
    host.vm.world.person = None;
    let before = host.vm.clone();
    let mut number = Numeric::new(CP);
    for _ in 0..32 {
        let Some(request) = number.advance().unwrap() else {
            break;
        };
        assert!(!matches!(request, NumericRequest::Friends { .. }));
        let Reply::Numeric(reply) = host
            .service(&Request::Numeric(request.clone()), number.state())
            .unwrap()
        else {
            unreachable!()
        };
        number.acknowledge(&request, reply).unwrap();
    }
    assert!(number.complete());
    assert_eq!(host.cp[&CP], b"0");
    assert_eq!(host.vm, before);
}

#[test]
fn unknown_widget_is_required_after_count_not_before_its_effects() {
    let all = cases();
    let mut host = Host::new(&all["cases"][0]["input"]);
    host.fields.label = None;
    let mut call = Call::new(VIEW, 1);
    for _ in 0..32 {
        match call.advance(Some(&host.fields)) {
            Err(error) => {
                assert_eq!(
                    error,
                    Error::FieldUnavailable {
                        owner: VIEW,
                        offset: 0x1d0
                    }
                );
                assert_eq!(host.cp[&CP], b"0");
                assert!(!host.vm.world.person.as_ref().unwrap().family_metadata[&1].dirty);
                let pending = call.clone();
                assert_eq!(call.advance(Some(&host.fields)), Err(error));
                assert_eq!(call, pending);
                host.fields.label = Some(WIDGET);
                assert!(matches!(
                    call.advance(Some(&host.fields)),
                    Ok(Some(Request::Window(WindowRequest::Equal { .. })))
                ));
                return;
            }
            Ok(Some(request)) => {
                let reply = host
                    .service(&request, call.numeric().map_or(0, Numeric::state))
                    .unwrap();
                call.acknowledge(&request, reply).unwrap();
            }
            _ => panic!("unknown widget incorrectly returned success"),
        }
    }
    panic!("label field read not reached")
}

#[test]
fn a_wrong_viewcontrol_owner_is_rejected_at_the_first_field_read() {
    let mut call = Call::new(VIEW, 1);
    let before = call.clone();
    assert_eq!(
        call.advance(Some(&Fields {
            owner: VIEW + 4,
            state: None,
            label: None
        })),
        Err(Error::FieldOwnerMismatch {
            expected: VIEW,
            actual: VIEW + 4
        })
    );
    assert_eq!(call, before);
    assert_eq!(
        call.advance(Some(&Fields {
            owner: VIEW,
            state: None,
            label: None
        })),
        Err(Error::FieldUnavailable {
            owner: VIEW,
            offset: 0xd4
        })
    );
    assert_eq!(call, before);
}

#[test]
fn supplying_a_previously_missing_owner_resumes_the_same_pending_getter() {
    let all = cases();
    let row = all["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["input"]["name"] == "after-compare-dirty-missing-neighborhood")
        .unwrap();
    let (mut pair, error, _) = run(&row["input"], true);
    assert!(error.is_some());
    let request = Request::Numeric(NumericRequest::Friends { family: FAMILY });
    assert_eq!(pair.runner.advance(None), Ok(Some(request.clone())));
    pair.host.vm.world.person.as_mut().unwrap().neighbor_slots = Some(vec![]);
    let reply = pair.host.service(&request, pair.runner.state()).unwrap();
    pair.runner.ack(&request, reply).unwrap();
    for _ in 0..32 {
        cold(&mut pair);
        let Some(r) = pair.runner.advance(Some(&pair.host.fields)).unwrap() else {
            break;
        };
        let p = pair.host.service(&r, pair.runner.state()).unwrap();
        pair.runner.ack(&r, p).unwrap();
    }
    assert!(pair.runner.complete());
    assert_eq!(pair.host.cp[&CP], b"0");
    assert_eq!(pair.host.widgets[&WIDGET], b"0");
    assert!(!pair.host.vm.world.person.as_ref().unwrap().family_metadata[&1].dirty);
    assert_eq!(pair.host.vm.tick, 88444);
}
