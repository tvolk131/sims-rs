//! Original wrapper + registry + RNG comparison with explicit callback draws.
use serde_json::{Value, json};
use sims_vm::{
    Vm, World,
    scheduler::{Inputs, Pass, Registry, Reply, Request},
};

struct Table {
    words: Vec<u32>,
    error: i16,
}
impl Registry for Table {
    fn len(&self) -> usize {
        self.words.len()
    }
    fn object(&self, slot: i32) -> Result<Option<u32>, String> {
        Ok(Some(slot as u32))
    }
    fn word(&self, slot: i32) -> Result<u32, String> {
        Ok(self.words[slot as usize - 1])
    }
    fn set_word(&mut self, slot: i32, word: u32) -> Result<(), String> {
        self.words[slot as usize - 1] = word;
        Ok(())
    }
    fn error(&self, object: u32) -> Result<i16, String> {
        Ok(if object == 1 { self.error } else { 0 })
    }
}
fn state(vm: &Vm, cursor: i32) -> Value {
    json!({"active":vm.rng_state,"lot":vm.lot_rng_state.unwrap(),"cursor":cursor,"tick":vm.tick})
}
fn restore(vm: &mut Vm) {
    let copy: Vm = serde_json::from_slice(&serde_json::to_vec(vm).unwrap()).unwrap();
    assert_eq!(*vm, copy);
    if let Some(scope) = &copy.lot_rng_scope {
        scope.validate(&copy).unwrap();
    }
    *vm = copy;
}
fn draws(vm: &mut Vm, n: u64, cursor: i32, events: &mut Vec<Value>) {
    for _ in 0..n {
        restore(vm);
        let result = sims_vm::rng::next(&mut vm.rng_state);
        events.push(json!({"event":"draw","result":result,"state":state(vm,cursor)}));
    }
    restore(vm);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_registry_rng_scope_returns_before_world_callbacks_even_on_false_passes() {
    let report: Value = serde_json::from_str(include_str!("fixtures/scheduler-rng.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 113);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut vm = Vm::new(
            World {
                scheduler_storage: Some(Default::default()),
                ..Default::default()
            },
            c["global"].as_u64().unwrap(),
        );
        vm.tick = 500;
        vm.lot_rng_state = Some(c["lot"].as_u64().unwrap() as u32);
        let mut table = Table {
            words: serde_json::from_value(c["words"].clone()).unwrap(),
            error: c["error"].as_i64().unwrap() as i16,
        };
        let mut cursor = c["cursor"].as_i64().unwrap() as i32;
        let input = Inputs {
            paused: c["paused"].as_i64().unwrap() as i16,
            mode: 0,
            lot_number: 7,
            tick: 500,
        };
        vm.begin_lot_rng_scope().unwrap();
        let before = vm.clone();
        assert!(vm.begin_lot_rng_scope().is_err());
        assert_eq!(vm, before);
        let mut events = vec![json!({"event":"pass","state":state(&vm,cursor)})];
        let mut pass = Pass::begin(input, &table, &mut cursor).unwrap();
        while let Some(request) = pass.advance(&mut table, &mut cursor, input).unwrap() {
            restore(&mut vm);
            pass = serde_json::from_slice(&serde_json::to_vec(&pass).unwrap()).unwrap();
            let (name, reply) = match request {
                Request::Tick { object, .. } => (
                    format!("tick{object}"),
                    Reply::Tick(if object == 1 {
                        c["returned"].as_u64().unwrap() as u8
                    } else {
                        1
                    }),
                ),
                Request::ObjectError { .. } => ("error".into(), Reply::Done),
            };
            events.push(json!({"event":name,"state":state(&vm,cursor)}));
            draws(&mut vm, c["draws"].as_u64().unwrap(), cursor, &mut events);
            pass.acknowledge(&request, reply).unwrap();
        }
        vm.end_lot_rng_scope().unwrap();
        let before = vm.clone();
        assert!(vm.end_lot_rng_scope().is_err());
        assert_eq!(vm, before);
        let complete = vm.finish_registry_pass(&pass).unwrap();
        events.push(
            json!({"event":if complete { "world0" } else { "world1" },"state":state(&vm,cursor)}),
        );
        draws(
            &mut vm,
            c["world_draws"].as_u64().unwrap(),
            cursor,
            &mut events,
        );
        assert_eq!(
            json!({"events":events,"state":state(&vm,cursor),"result":complete,"words":table.words}),
            row["original"],
            "native RNG scope case {n}"
        );
    }
}
