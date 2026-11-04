use serde_json::{Value, json};
use sims_sim::pose::material_texture::{Attach, Reply, Request, State};

fn state(texture: Option<u64>) -> State {
    State {
        words: std::array::from_fn(|n| n as u32),
        texture,
        device: Some(10),
        handles: [(1, 101), (2, 202), (3, 303)].into(),
    }
}

fn next(attach: &mut Attach, state: &mut State) -> Option<Request> {
    let before = (attach.clone(), state.clone());
    (*attach, *state) = serde_json::from_value(serde_json::to_value(&before).unwrap()).unwrap();
    assert_eq!((attach.clone(), state.clone()), before);
    let request = attach.advance(state).unwrap();
    let pending = (attach.clone(), state.clone());
    assert_eq!(attach.advance(state).unwrap(), request);
    assert_eq!((attach.clone(), state.clone()), pending);
    (*attach, *state) = serde_json::from_value(serde_json::to_value(&pending).unwrap()).unwrap();
    request
}

#[test]
fn replacement_acquires_before_releasing_even_for_the_same_texture() {
    for old in [None, Some(1), Some(2)] {
        for new in [None, Some(1), Some(2)] {
            let mut state = state(old);
            let mut attach = Attach::new(new);
            let mut events = vec![];
            while let Some(request) = next(&mut attach, &mut state) {
                let reply = match request {
                    Request::Acquire { texture } => {
                        assert_eq!(state.texture, old);
                        events.push(("acquire", texture));
                        Reply::Done
                    }
                    Request::Release { texture } => {
                        assert_eq!(state.texture, old);
                        events.push(("release", texture));
                        Reply::Done
                    }
                    Request::Publish { device, words } => {
                        assert_eq!(device, 10);
                        assert_eq!(state.texture, new);
                        assert_eq!(words[18], new.map_or(0, |id| state.handles[&id]));
                        let mut expected = std::array::from_fn(|n| n as u32);
                        expected[18] = words[18];
                        assert_eq!(words, expected);
                        Reply::Published(0)
                    }
                };
                attach.acknowledge(&request, reply).unwrap();
            }
            assert_eq!(attach.result(), Some(0));
            assert_eq!(
                events,
                new.map(|id| ("acquire", id))
                    .into_iter()
                    .chain(old.map(|id| ("release", id)))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn callbacks_change_live_owner_handle_and_device_without_changing_captured_replacement() {
    let mut state = state(Some(2));
    let mut attach = Attach::new(Some(1));
    let request = next(&mut attach, &mut state).unwrap();
    assert_eq!(request, Request::Acquire { texture: 1 });
    state.texture = Some(3);
    attach.acknowledge(&request, Reply::Done).unwrap();
    let request = next(&mut attach, &mut state).unwrap();
    assert_eq!(request, Request::Release { texture: 3 });
    state.texture = Some(2);
    state.device = Some(42);
    state.handles.insert(1, 0x12345678);
    attach.acknowledge(&request, Reply::Done).unwrap();
    let request = next(&mut attach, &mut state).unwrap();
    assert_eq!(state.texture, Some(1));
    assert_eq!(state.words[18], 0x12345678);
    assert_eq!(
        request,
        Request::Publish {
            device: 42,
            words: state.words
        }
    );
    // The original returns a failed HRESULT without rolling back writes or
    // overwriting any additional changes made by the publication callback.
    state.texture = Some(3);
    state.words[4] = 0xabcdef01;
    attach
        .acknowledge(&request, Reply::Published(0x80004005))
        .unwrap();
    let before = state.clone();
    assert_eq!(next(&mut attach, &mut state), None);
    assert_eq!(state, before);
    assert_eq!(attach.result(), Some(0x80004005));
}

#[test]
fn wrong_or_stale_reply_keeps_the_continuation_unchanged() {
    let mut state = state(None);
    let mut attach = Attach::new(Some(1));
    let request = next(&mut attach, &mut state).unwrap();
    let before = attach.clone();
    assert!(attach.acknowledge(&request, Reply::Published(0)).is_err());
    assert_eq!(attach, before);
    assert!(
        attach
            .acknowledge(&Request::Acquire { texture: 2 }, Reply::Done)
            .is_err()
    );
    assert_eq!(attach, before);
    attach.acknowledge(&request, Reply::Done).unwrap();
    let before = attach.clone();
    assert!(attach.acknowledge(&request, Reply::Done).is_err());
    assert_eq!(attach, before);
}

#[test]
fn missing_texture_or_device_requires_its_real_backend() {
    let mut state = state(None);
    let mut attach = Attach::new(Some(1));
    let request = next(&mut attach, &mut state).unwrap();
    attach.acknowledge(&request, Reply::Done).unwrap();
    state.handles.remove(&1);
    assert!(attach.advance(&mut state).unwrap_err().contains("handle"));
    assert_eq!(state.texture, Some(1));
    assert_eq!(state.words[18], 18); // No fabricated texture/white fallback.
    assert_eq!(attach.result(), None);

    let mut state = self::state(None);
    state.device = None;
    let mut attach = Attach::new(None);
    assert!(
        attach
            .advance(&mut state)
            .unwrap_err()
            .contains("interface")
    );
    assert_eq!(state.words[18], 0); // Original writes precede the device call.
    assert_eq!(attach.result(), None);
}

fn patch(state: &mut State, changes: &Value) {
    if let Some(v) = changes.get("texture") {
        state.texture = v.as_u64();
    }
    if let Some(v) = changes.get("device") {
        state.device = v.as_u64();
    }
    if let Some(v) = changes.get("handles") {
        for (key, value) in v.as_object().unwrap() {
            state
                .handles
                .insert(key.parse().unwrap(), value.as_u64().unwrap() as u32);
        }
    }
}

fn run(case: &Value) -> Value {
    let mut state: State = serde_json::from_value(case["state"].clone()).unwrap();
    let mut attach = Attach::new(case["replacement"].as_u64());
    let mut events = vec![];
    while let Some(request) = next(&mut attach, &mut state) {
        events.push(json!({"request": request,"state":state}));
        let (kind, reply) = match request {
            Request::Acquire { .. } => ("Acquire", Reply::Done),
            Request::Release { .. } => ("Release", Reply::Done),
            Request::Publish { .. } => (
                "Publish",
                Reply::Published(case["publication_result"].as_u64().unwrap() as u32),
            ),
        };
        for change in case["mutations"].as_array().unwrap() {
            if change["after"] == kind {
                patch(&mut state, &change["patch"]);
            }
        }
        attach.acknowledge(&request, reply).unwrap();
    }
    json!({"state":state,"events":events,"result":attach.result()})
}

#[test]
#[ignore = "synthetic inputs from native_material_texture.py --inputs-only; not native evidence"]
fn synthetic_matrix_restores_at_every_callback() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/material-texture-inputs.json")).unwrap(),
    )
    .unwrap();
    let rows = inputs["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 108);
    let output = json!({"scope":"Rust callback/restore checks only; no native comparison", "cases":rows.iter().map(|c|json!({"input":c,"rust":run(c)})).collect::<Vec<_>>()});
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("cache/material-texture-rust.json"))
        .unwrap();
    serde_json::to_writer(file, &output).unwrap();
}

#[test]
#[ignore = "requires freshly executed guarded original texture attachment probe"]
fn texture_attachment_matches_original_callbacks_state_and_hresult() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let evidence: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/material-texture.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        evidence["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(
        evidence["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    let rows = evidence["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 108);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            run(&row["input"]),
            row["original"],
            "native texture attachment case {i}"
        );
    }
}
