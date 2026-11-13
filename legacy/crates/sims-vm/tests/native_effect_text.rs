use serde_json::{Value, json};
use sims_vm::effect::{self, Emission, Host, Outcome};

struct Services<'a> {
    input: &'a Value,
    events: Vec<Value>,
}
impl Services<'_> {
    fn number(&self, key: &str) -> i64 {
        self.input[key].as_i64().unwrap()
    }
    fn boolean(&self, key: &str) -> bool {
        self.input[key].as_bool().unwrap()
    }
}
impl Host for Services<'_> {
    fn redraw(&mut self) -> Result<(), String> {
        self.events.push(json!(["redraw", 11, 1]));
        Ok(())
    }
    fn target(&mut self) -> Result<i16, String> {
        let target = self.number("target") as i16;
        if target == 0 {
            Err("23".into())
        } else {
            Ok(target)
        }
    }
    fn duration(&mut self, local: bool, index: i16) -> Result<i16, String> {
        if local {
            usize::try_from(index)
                .ok()
                .and_then(|i| [-7, 31, 32767, -32768].get(i).copied())
                .ok_or("8".into())
        } else {
            Ok(index)
        }
    }
    fn active(&mut self) -> Result<bool, String> {
        self.events.push(json!(["view"]));
        Ok(self.boolean("visible"))
    }
    fn residential(&mut self) -> Result<bool, String> {
        self.events.push(json!(["live_family"]));
        Ok(self.boolean("live_family"))
    }
    fn actor_family_permits(&mut self) -> Result<bool, String> {
        Ok(self.number("kind") == 2
            && (self.number("family") == self.number("controlled_family")
                || self.number("controlled_family") == 0 && self.number("special_family") != 0))
    }
    fn notify(&mut self) -> Result<(), String> {
        self.events.push(json!(["notify", 268, 0]));
        Ok(())
    }
    fn text(&mut self, entry: u8) -> Result<String, String> {
        let bhav = self.number("bhav") as u16;
        self.events.push(json!([
            "resource",
            if bhav == u16::MAX {
                bhav
            } else {
                bhav & 0x7fff
            }
        ]));
        if !self.boolean("resource") {
            return Ok(String::new());
        }
        self.events.push(json!(["load", 305, 0]));
        self.events.push(json!(["entry", entry, u32::MAX]));
        Ok(self.input["text"].as_str().unwrap_or_default().into())
    }
    fn format(&mut self, text: String) -> Result<String, String> {
        self.events.push(json!(["format", text, 0, 0]));
        Ok(self.input["formatted"].as_str().unwrap().into())
    }
    fn emit(&mut self, e: Emission) -> Result<(), String> {
        self.events.push(json!([
            "emit",
            [
                e.flags & 1,
                e.object,
                i32::from(e.flags & 8 == 0),
                e.duration_ms,
                e.kind,
                e.size,
                i32::from(e.flags & 4 == 0),
                (e.flags >> 3) & 1,
                (e.flags >> 4) & 1
            ],
            e.text
        ]));
        Ok(())
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_effect_text_gates_order_and_failure_continuations() {
    let report: Value = serde_json::from_str(include_str!("fixtures/effect-text.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut host = Services {
            input: c,
            events: vec![],
        };
        let mut state = host.number("state") as u32;
        let mut operands = [0; 8];
        operands[..2].copy_from_slice(&(host.number("duration") as i16).to_le_bytes());
        for (n, key) in [(2, "size"), (3, "effect"), (4, "flags"), (5, "entry")] {
            operands[n] = host.number(key) as u8;
        }
        let result = effect::step(&mut state, operands, &mut host);
        let (error, returned) = match result {
            Ok(Outcome::Yield) => (None, Some(2)),
            Ok(Outcome::Complete) => (None, Some(1)),
            Err(error) => (Some(error.parse::<u32>().unwrap()), None),
        };
        assert_eq!(
            json!({"error":error,"returned":returned,"state":state,"events":host.events}),
            *expected,
            "case {i}: {c}"
        );
    }
}

fn session(caption: Option<&str>, entry: u8) -> (sims_vm::Program, sims_vm::Vm, u64) {
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_vm::*;
    use std::collections::BTreeMap;
    let mut private = Module {
        source: "private".into(),
        semiglobal: Some("semi".into()),
        ..Default::default()
    };
    private
        .strings
        .insert(305, vec!["WRONG PRIVATE RESOURCE".into()]);
    private.catalog_names.insert(128, "Frankie".into());
    private.catalog_names.insert(129, "Chair".into());
    let mut definition_words = vec![0; 10];
    definition_words[9] = 2;
    private
        .object_definition_words
        .insert(128, definition_words);
    let mut semi = Module {
        source: "semi".into(),
        ..Default::default()
    };
    if let Some(caption) = caption {
        semi.strings.insert(305, vec![caption.into()]);
    }
    semi.routines.insert(
        8305,
        Routine {
            name: "effect".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 3,
                instructions: vec![Instruction {
                    opcode: 0x23,
                    on_true: 254,
                    on_false: 253,
                    operands: [10, 0, 1, 2, 25, entry, 0, 0],
                }],
            },
        },
    );
    let p = Program {
        modules: BTreeMap::from([("private".into(), private), ("semi".into(), semi)]),
        ..Default::default()
    };
    let mut world = World::default();
    world.actions = Some(action::WorldState {
        live_family: true,
        ..Default::default()
    });
    for id in [1, 2] {
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "private".into(),
                    object: 127 + id as u16,
                    guid: id as u32,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
    }
    let mut vm = Vm::new(world, 1);
    let id = vm
        .spawn(
            &p,
            Start {
                owner: "private",
                bhav: 8305,
                caller: 1,
                callee: 1,
                stack_object: 2,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (p, vm, id)
}

#[test]
fn live_effect_captions_use_behavior_resource_and_restore_each_phase() {
    use sims_vm::{Snapshot, Status, Stop};
    for (text, entry, expected) in [
        (
            Some("$Me has burned to death!"),
            1,
            "Frankie has burned to death!",
        ),
        (Some("$Me: $Object / $Local:0"), 1, "Frankie: Chair / 0"),
        (Some("Raw text"), 1, "Raw text"),
        (Some(""), 1, ""),
        (None, 1, ""),
        (Some("unused"), 0, ""),
        (Some("unused"), 255, ""),
    ] {
        let (p, mut vm, id) = session(text, entry);
        let mut resumed = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        for phase in 0..3 {
            let actual = vm.run_slice(&p, id, 1).unwrap();
            assert_eq!(actual, resumed.run_slice(&p, id, 1).unwrap());
            assert_eq!(vm, resumed);
            if phase < 2 {
                assert_eq!(actual.stop, Stop::Sleeping);
            } else {
                assert_eq!(vm.threads[&id].status, Status::Returned(true));
            }
            resumed = serde_json::from_slice::<Snapshot>(
                &serde_json::to_vec(&Snapshot::capture(&p, &vm)).unwrap(),
            )
            .unwrap()
            .restore(&p)
            .unwrap();
            if phase < 2 {
                vm.advance_tick().unwrap();
                resumed.advance_tick().unwrap();
            }
        }
        assert_eq!(vm.effects.len(), 1);
        assert_eq!(vm.effects[0].text, expected);
        assert_eq!(vm.effects[0].object, 2);
        assert_eq!(vm.effects[0].duration_ms, 10000);
    }
}

#[test]
fn live_effect_failures_retain_native_phase_and_suppression_does_not_yield_again() {
    use sims_vm::{Snapshot, Status, Stop};
    let (p, mut vm, id) = session(Some("$Me"), 1);
    vm.run_slice(&p, id, 1).unwrap();
    vm.advance_tick().unwrap();
    vm.world.objects.remove(&2);
    assert_eq!(vm.run_slice(&p, id, 1).unwrap().stop, Stop::Faulted);
    assert_eq!(vm.threads[&id].stack[0].continuation, 2);
    assert!(vm.effects.is_empty());
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);

    for local in [-1i16, 3] {
        let (mut p, mut vm, id) = session(Some("$Unsupported:0"), 1);
        let operands = &mut p
            .modules
            .get_mut("semi")
            .unwrap()
            .routines
            .get_mut(&8305)
            .unwrap()
            .body
            .instructions[0]
            .operands;
        operands[..2].copy_from_slice(&local.to_le_bytes());
        operands[4] = 32;
        // Duration resolution precedes even the inactive-view gate.
        vm.world.special_effects.active = false;
        vm.run_slice(&p, id, 1).unwrap();
        vm.advance_tick().unwrap();
        let mut restored = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        let result = vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(result.stop, Stop::Faulted);
        assert_eq!(result, restored.run_slice(&p, id, 1).unwrap());
        assert_eq!(vm, restored);
        assert_eq!(vm.threads[&id].stack[0].continuation, 2);
        assert!(vm.effects.is_empty());
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }

    let (mut p, mut vm, id) = session(Some("$Unsupported:0"), 1);
    p.modules
        .get_mut("semi")
        .unwrap()
        .routines
        .get_mut(&8305)
        .unwrap()
        .body
        .instructions[0]
        .operands[4] = 0;
    vm.world.special_effects.active = false;
    vm.run_slice(&p, id, 1).unwrap();
    vm.advance_tick().unwrap();
    vm.run_slice(&p, id, 1).unwrap();
    assert_eq!(vm.threads[&id].status, Status::Returned(true));
    assert!(
        vm.effects.is_empty(),
        "suppressed effect must not load/format its text"
    );
}

#[test]
fn live_effect_gates_use_zoning_actor_type_and_controlled_family() {
    use sims_vm::{Snapshot, Status, Stop, person};
    // (lot, zoning, kind, person present, family, controlled, special, flags,
    // active view, emits). These exercise the production adapter, not the
    // fixture Host, including zoning taking precedence over authored context.
    for (lot, zone, kind, person_present, family, controlled, special, flags, active, emits) in [
        (9, 1, 4, false, 7, 3, 0, 0, true, true),
        (0, 0, 4, false, 7, 3, 0, 0, true, true),
        (9, 2, 2, true, 3, 3, 0, 0, true, true),
        (9, 2, 2, true, 7, 3, 0, 0, true, false),
        (9, 2, 4, true, 3, 3, 0, 0, true, false),
        (9, 2, 2, false, 3, 3, 0, 0, true, false),
        (9, 2, 2, true, 7, 0, 1, 0, true, true),
        (9, 2, 2, true, 7, 0, 0, 0, true, false),
        (9, 2, 2, true, 0, 0, 0, 0, true, true),
        (9, 2, 2, true, 7, 3, 1, 0, true, false),
        (9, 0, 2, true, 7, 3, 0, 0, true, false),
        (9, 1, 2, true, 3, 3, 0, 0, false, false),
        (9, 2, 4, false, 7, 3, 0, 8, false, true),
        (9, 2, 2, true, 3, 3, 0, 64, true, true),
    ] {
        let (mut p, mut vm, id) = session(Some("$Me"), 1);
        p.modules
            .get_mut("private")
            .unwrap()
            .object_definition_words
            .get_mut(&128)
            .unwrap()[9] = kind;
        p.modules
            .get_mut("semi")
            .unwrap()
            .routines
            .get_mut(&8305)
            .unwrap()
            .body
            .instructions[0]
            .operands[4] = flags;
        vm.world.special_effects.active = active;
        vm.world.globals.insert(10, lot);
        vm.world.person = Some(person::WorldState {
            controlled_family: controlled,
            lot_zoning: [(9, zone)].into(),
            ..Default::default()
        });
        vm.world.objects.get_mut(&1).unwrap().person =
            person_present.then(|| [(61, family), (77, special)].into());
        vm.run_slice(&p, id, 1).unwrap();
        vm.advance_tick().unwrap();
        let mut restored = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        let result = vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(result, restored.run_slice(&p, id, 1).unwrap());
        assert_eq!(vm, restored);
        assert_eq!(vm.effects.len(), usize::from(emits));
        if emits {
            assert_eq!(result.stop, Stop::Sleeping);
            assert_eq!(vm.threads[&id].stack[0].continuation, 2);
            assert_eq!(vm.effects[0].text, "Frankie");
        } else {
            assert_eq!(vm.threads[&id].status, Status::Returned(true));
        }
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}
