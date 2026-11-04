use serde_json::{Value, json};
use sims_vm::{
    ObjectId,
    deletion_entry::{Entry, Request, View},
};

struct State {
    flags: u32,
    owner: Option<ObjectId>,
    selected: Option<ObjectId>,
}
impl View for State {
    fn flags(&self, _: ObjectId) -> Result<u32, String> {
        Ok(self.flags)
    }
    fn write_flags(&mut self, _: ObjectId, flags: u32) -> Result<(), String> {
        self.flags = flags;
        Ok(())
    }
    fn ui_owner(&self) -> Option<ObjectId> {
        self.owner
    }
    fn selected(&self) -> Result<Option<ObjectId>, String> {
        Ok(self.selected)
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_deletion_entry_rereads_state_and_restores_at_every_service() {
    let report: Value = serde_json::from_str(include_str!("fixtures/deletion-entry.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 144);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut state = State {
            flags: c["flags"].as_u64().unwrap() as u32,
            owner: [None, Some(2), Some(1)][c["owner"].as_u64().unwrap() as usize],
            selected: [None, Some(2), Some(1)][c["selected"].as_u64().unwrap() as usize],
        };
        let mut e = Entry::new(2);
        let mut events = vec![];
        while let Some(r) = e.advance(&mut state).unwrap() {
            e = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
            assert_eq!(e.advance(&mut state).unwrap(), Some(r));
            let name = match r {
                Request::Signal => "signal",
                Request::ReleaseUi => "ui",
                Request::ClearSelection => "selection",
                Request::ReleaseIndicator => "indicator",
                Request::Invalidate => "invalidate",
                Request::References => "references",
            };
            events.push(json!([name, state.flags]));
            match r {
                Request::Signal if c["signal_mutation"] == true => {
                    state.flags = 0x8002;
                    state.owner = Some(2);
                    state.selected = Some(2);
                }
                Request::ReleaseUi => {
                    state.owner = None;
                    if c["ui_mutation"] == true {
                        state.selected = if state.selected == Some(2) {
                            None
                        } else {
                            Some(2)
                        };
                    }
                }
                Request::ClearSelection => state.selected = None,
                _ => (),
            }
            let wrong = if r == Request::Signal {
                Request::Invalidate
            } else {
                Request::Signal
            };
            let unchanged = e.clone();
            assert!(e.acknowledge(wrong).is_err());
            assert_eq!(e, unchanged);
            e.acknowledge(r).unwrap();
        }
        assert!(e.complete());
        events.push(json!(["reset", state.flags]));
        assert_eq!(
            json!({"events":events,"flags":state.flags}),
            row["original"],
            "{c}"
        );
    }
}
