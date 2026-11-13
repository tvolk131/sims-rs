use serde_json::{Value, json};
use sims_vm::{
    ObjectId,
    load_completion::{Completion, Inputs, Reply, Request, View},
};
use std::collections::BTreeSet;

#[test]
fn idle_application_context_records_no_broadcast_and_rejects_advancement() {
    use sims_vm::commander::{
        Registry,
        house::{Fields, Interface, Receiver, dispatch::Dispatch},
    };
    let mut registry = Registry::default();
    registry.construct(1).unwrap();
    registry.bind(1, Receiver::Globals).unwrap();
    let fields = Fields {
        viewer_interface: false,
        include_visible: 0,
        renderer_dirty: 0,
        ui: Interface {
            mask_owner_present: false,
            panel_active: false,
            shared_present: false,
            gated: 0,
            creation_gated: 1,
            dirty: 0,
        },
    };
    let mut d = Dispatch::idle(registry, fields).unwrap();
    let mut globals = std::collections::BTreeMap::from([(3, 7), (26, 1)]);
    let before = (d.clone(), globals.clone());
    assert!(d.is_idle());
    assert_eq!(d.root_message(), None);
    assert_eq!(d.returned(), None);
    assert_eq!(d.pending(), None);
    assert!(
        d.advance(&mut globals, false, 1, |_, _, _, _| panic!(
            "idle emitted an event"
        ))
        .is_err()
    );
    assert!(d.nest(0xf6, 0).is_err());
    assert!(d.finish_nested().is_err());
    assert_eq!((d.clone(), globals), before);
    d = serde_json::from_slice(&serde_json::to_vec(&d).unwrap()).unwrap();
    d.validate().unwrap();
    assert!(d.is_idle());
}
struct Host {
    input: Value,
    order: Vec<ObjectId>,
    people: Value,
    family_flags: u32,
    family_id: i32,
    manager_lot: i32,
}
impl Host {
    fn new(v: &Value) -> Self {
        Self {
            input: v.clone(),
            order: serde_json::from_value(v["order"].clone()).unwrap(),
            people: v["people"].clone(),
            family_flags: v["family_flags"].as_u64().unwrap() as u32,
            family_id: v["family_id"].as_i64().unwrap() as i32,
            manager_lot: v["manager_lot"].as_i64().unwrap() as i32,
        }
    }
    fn state(&self) -> Value {
        json!({"order":self.order,"people":self.people,"family_flags":self.family_flags,"family_id":self.family_id as u32,"manager_lot":self.manager_lot as u32})
    }
    fn patch(&mut self, v: &Value) {
        if let Some(n) = v["family_flags"].as_u64() {
            self.family_flags = n as u32;
        }
        if let Some(n) = v["family_id"].as_i64() {
            self.family_id = n as i32;
        }
        if let Some(n) = v["manager_lot"].as_i64() {
            self.manager_lot = n as i32;
        }
        if !v["order"].is_null() {
            self.order = serde_json::from_value(v["order"].clone()).unwrap();
        }
        if !v["people"].is_null() {
            self.people = v["people"].clone();
        }
    }
}
impl View for Host {
    fn people(&self) -> &[ObjectId] {
        &self.order
    }
    fn resource_flags(&self, id: ObjectId) -> Result<u32, String> {
        Ok(self.people[id as usize - 1]["flags"].as_u64().unwrap() as u32)
    }
    fn person(&self, id: ObjectId, field: i16) -> Result<i16, String> {
        Ok(self.people[id as usize - 1][match field {
            32 => "visitor",
            61 => "family",
            67 => "home",
            _ => panic!(),
        }]
        .as_i64()
        .unwrap() as i16)
    }
    fn manager_lot(&self) -> i32 {
        self.manager_lot
    }
    fn controller_suppressed(&self) -> u32 {
        self.input["controller_suppressed"].as_u64().unwrap() as u32
    }
    fn mode(&self) -> i16 {
        self.input["mode"].as_i64().unwrap() as i16
    }
    fn controller_gate(&self) -> Result<i16, String> {
        Ok(self.input["controller_gate"].as_i64().unwrap() as i16)
    }
    fn family_id(&self) -> i32 {
        self.family_id
    }
    fn family_flags(&self) -> Result<u32, String> {
        Ok(self.family_flags)
    }
    fn write_family_flags(&mut self, value: u32) -> Result<(), String> {
        self.family_flags = value;
        Ok(())
    }
    fn family_home(&self, id: i32) -> Result<Option<i16>, String> {
        Ok(self.input["families"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"].as_i64() == Some(id as i64))
            .map(|f| f["home"].as_i64().unwrap() as i16))
    }
}
fn call(r: Request) -> Value {
    match r {
        Request::Delete(id) => json!(["delete", id]),
        r => json!([
            match r {
                Request::FindController => "find_controller",
                Request::LookupController => "lookup_controller",
                Request::CreateController => "create_controller",
                Request::ResetBudget => "reset_budget",
                Request::ResetStatistics => "reset_statistics",
                Request::ResetGlobals => "reset_globals",
                Request::ResetWorld => "reset_world",
                Request::ResetLot => "reset_lot",
                Request::RefreshVisitors => "refresh_visitors",
                Request::ImportLot => "import_lot",
                Request::Environment => "environment",
                Request::HouseOpen => "house_open",
                Request::ReleaseSnapshot => "release_snapshot",
                _ => unreachable!(),
            },
            0
        ]),
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_load_cleanup_gates_live_rescans_and_flags_survive_every_callback() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/load-completion.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 192);
    for c in fixture["cases"].as_array().unwrap() {
        let input = &c["input"];
        let mut host = Host::new(input);
        let mut state = Completion::new(Inputs {
            lot_argument: input["lot_argument"].as_i64().unwrap() as i32,
            reset_requested: input["reset_requested"].as_u64().unwrap() as u8,
            imported: input["imported"].as_u64().unwrap() as u8,
            residents_before_visits: serde_json::from_value(input["fresh"].clone()).unwrap(),
        })
        .unwrap();
        let mut events = vec![];
        let mut applied = BTreeSet::new();
        while let Some(request) = state.advance(&mut host).unwrap() {
            state = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            let before = (state.clone(), host.state());
            assert_eq!(state.advance(&mut host).unwrap(), Some(request));
            let wrong = if request == Request::FindController {
                Request::Environment
            } else {
                Request::FindController
            };
            assert!(state.acknowledge(wrong, Reply::Done).is_err());
            assert_eq!((state.clone(), host.state()), before);
            let call = call(request);
            let mut event = host.state();
            event["call"] = call.clone();
            events.push(event);
            for (i, change) in input["changes"].as_array().unwrap().iter().enumerate() {
                if change["at"] == call && applied.insert(i) {
                    host.patch(change);
                }
            }
            let reply = match request {
                Request::Delete(id) => {
                    host.order.retain(|n| *n != id);
                    Reply::Done
                }
                Request::FindController => Reply::Found(input["existing"].as_bool().unwrap()),
                Request::LookupController => Reply::Found(input["available"].as_bool().unwrap()),
                Request::RefreshVisitors => {
                    for (i, v) in input["visitor_values"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .enumerate()
                    {
                        host.people[i]["visitor"] = v.clone();
                    }
                    Reply::Done
                }
                _ => Reply::Done,
            };
            state = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            state.acknowledge(request, reply).unwrap();
            assert!(state.acknowledge(request, reply).is_err());
        }
        assert!(state.complete());
        let mut actual = host.state();
        actual["events"] = json!(events);
        actual["applied"] = json!(applied);
        actual["result"] = json!(0);
        assert_eq!(actual, c["original"], "{input}");
    }
}
