//! Differential check of private original-code reports, with no original bytes embedded.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Definition, Module, Object, Program, Routine, Start, Status, Vm, World};
use std::{collections::BTreeMap, fs};
fn routine(
    format: u16,
    kind: u8,
    args: u8,
    locals: u16,
    instructions: Vec<Instruction>,
) -> Routine {
    Routine {
        name: "synthetic".into(),
        body: Behavior {
            format,
            kind,
            args,
            locals,
            instructions,
        },
    }
}
fn ins() -> Instruction {
    Instruction {
        opcode: 2,
        on_true: 254,
        on_false: 255,
        operands: [0; 8],
    }
}
fn fixture(stack_object: i16) -> (Program, Vm) {
    let mut module = Module {
        source: "fixture".into(),
        ..Module::default()
    };
    module
        .routines
        .insert(4096, routine(0x8002, 0, 4, 4, vec![ins()]));
    let mut world = World::default();
    for n in 1..=2i16 {
        let mut words: Vec<i16> = (700 + n * 100..808 + n * 100).collect();
        words[9] = 2;
        module.object_definition_words.insert(n as u16, words);
        world.objects.insert(
            n,
            Object {
                definition: Some(Definition {
                    source: "fixture".into(),
                    object: n as u16,
                    guid: 0,
                    slots: 0,
                }),
                attributes: (n * 100..n * 100 + 4).collect(),
                dynamic_sprite_flags: (n * 1000..n * 1000 + 4).collect(),
                data: (0..72).map(|i| (i, n * 10 + i)).collect(),
                motives: Some([0; 16]),
                person: Some((0..256).map(|i| (i, n * 100 + i)).collect()),
                temps: if n == 1 {
                    [1, 2, 3, 0, 7, -1, 8, 255]
                } else {
                    [201, 202, 203, 204, 205, 206, 207, 208]
                },
                slots: vec![None, Some(n)],
                ..Object::default()
            },
        );
    }
    world.globals = (0..64).map(|i| (i, 300 + i)).collect();
    world.neighborhood = (0..16).map(|i| (i, 600 + i)).collect();
    world.tree_advertisements = (0..16)
        .map(|i| [400 + i * 3, 401 + i * 3, 402 + i * 3])
        .collect();
    let program = Program {
        globals: "fixture".into(),
        modules: BTreeMap::from([("fixture".into(), module)]),
        ..Program::default()
    };
    let mut vm = Vm::new(world, 1);
    vm.spawn(
        &program,
        Start {
            owner: "fixture",
            bhav: 4096,
            caller: 1,
            callee: 2,
            stack_object,
            args: vec![1, 2, 3, 4],
            check: false,
        },
    )
    .unwrap();
    vm.threads.get_mut(&1).unwrap().stack[0].locals = vec![501, 502, 503, 504];
    (program, vm)
}
fn error_code(message: &str) -> u32 {
    message
        .split("native error 0x")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .and_then(|s| u32::from_str_radix(s, 16).ok())
        .unwrap_or(u32::MAX)
}
fn main() {
    let report: Value = serde_json::from_slice(
        &fs::read(std::env::args().nth(1).expect("private report path")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["schema"], 1);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let mut failures = 0;
    let mut count = 0;
    for row in report["memory"].as_array().unwrap() {
        let i = &row["input"];
        let (p, mut vm) = fixture(i["stack_object"].as_i64().unwrap_or(2) as i16);
        if let Some(v) = i["temp0"].as_i64() {
            vm.world.objects.get_mut(&1).unwrap().temps[0] = v as i16;
        }
        if let Some(v) = i["arg0"].as_i64() {
            vm.threads.get_mut(&1).unwrap().stack[0].args[0] = v as i16;
        }
        let actual = match vm.inspect_memory(
            &p,
            1,
            i["scope"].as_u64().unwrap() as u8,
            i["index"].as_i64().unwrap() as i16,
            i["write"].as_bool().unwrap(),
        ) {
            Ok(v) => {
                json!({"value":v.value,"writable":v.writable,"error":null,"float_pointer":v.float_pointer})
            }
            Err(e) => {
                json!({"value":null,"writable":false,"error":error_code(&e),"float_pointer":false})
            }
        };
        count += 1;
        if actual != row["original"] {
            failures += 1;
            if failures <= 20 {
                eprintln!("memory {i}: Rust {actual}; original {}", row["original"]);
            }
        }
    }
    for row in report["motives"].as_array().unwrap() {
        let i = &row["input"];
        let (mut p, mut vm) = fixture(2);
        let lhs = i["lhs"].as_f64().unwrap() as f32;
        vm.world.objects.get_mut(&1).unwrap().motives = Some([lhs.to_bits(); 16]);
        let mut operands = [0; 8];
        operands[2..4].copy_from_slice(&(i["rhs"].as_i64().unwrap() as i16).to_le_bytes());
        operands[5] = i["operator"].as_u64().unwrap() as u8;
        operands[6] = 14;
        operands[7] = 7;
        p.modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions[0]
            .operands = operands;
        vm.run_slice(&p, 1, 1).unwrap();
        let (error, returned) = match &vm.threads[&1].status {
            Status::Returned(b) => (None, Some(*b)),
            Status::Faulted(f) => (Some(error_code(&f.message)), None),
            s => panic!("unexpected {s:?}"),
        };
        let actual = json!({"error":error,"returned":returned,"bits":vm.world.objects[&1].motives.unwrap()[0]});
        count += 1;
        if actual != row["original"] {
            failures += 1;
            if failures <= 20 {
                eprintln!("motive {i}: Rust {actual}; original {}", row["original"]);
            }
        }
    }
    for row in report["assignments"].as_array().unwrap() {
        let i = &row["input"];
        let scope = i["scope"].as_u64().unwrap() as u8;
        let index = i["index"].as_i64().unwrap() as i16;
        let (mut p, mut vm) = fixture(2);
        let mut operands = [0; 8];
        operands[..2].copy_from_slice(&index.to_le_bytes());
        operands[2..4].copy_from_slice(&(-321i16).to_le_bytes());
        operands[5..].copy_from_slice(&[5, scope, 7]);
        let body = &mut p
            .modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body;
        body.instructions[0] = Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands,
        };
        body.instructions.push(ins());
        let before = vm.inspect_memory(&p, 1, scope, index, true).unwrap();
        vm.run_slice(&p, 1, 1).unwrap();
        let (error, returned) = match &vm.threads[&1].status {
            Status::Ready => (None, Some(true)),
            Status::Faulted(f) => (Some(error_code(&f.message)), None),
            s => panic!("unexpected {s:?}"),
        };
        // Native report observes the already-resolved cell even if assigning changes its selector.
        let value = if before.writable {
            Some(if error.is_none() { -321 } else { before.value })
        } else {
            None
        };
        if before.writable && error.is_none() {
            assert_eq!(
                vm.inspect_memory(&p, 1, scope, index, false).unwrap().value,
                -321
            );
        }
        let actual = json!({"error":error,"returned":returned,"value":value});
        count += 1;
        if actual != row["original"] {
            failures += 1;
            if failures <= 20 {
                eprintln!(
                    "assignment {i}: Rust {actual}; original {}",
                    row["original"]
                );
            }
        }
    }
    for row in report["calls"].as_array().unwrap() {
        let i = &row["input"];
        let (mut p, mut vm) = fixture(2);
        let words: Vec<i16> = i["operands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i16)
            .collect();
        let operands: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        p.modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions[0] = Instruction {
            opcode: 4097,
            on_true: 254,
            on_false: 255,
            operands: operands.try_into().unwrap(),
        };
        p.modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .insert(4097, routine(0x8002, 0, 4, 0, vec![ins()]));
        vm.run_slice(&p, 1, 1).unwrap();
        let actual = json!({"error":null,"args":vm.threads[&1].stack.last().unwrap().args});
        count += 1;
        if actual != row["original"] {
            failures += 1;
            if failures <= 20 {
                eprintln!("call {i}: Rust {actual}; original {}", row["original"]);
            }
        }
    }
    for row in report["frames"].as_array().unwrap() {
        let i = &row["input"];
        let (mut p, mut vm) = fixture(2);
        let parent = p
            .modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap();
        parent.body.args = 0;
        parent.body.locals = 0;
        let mut words = vec![i["entry"].as_i64().unwrap() as i16, -1, 123, -32768];
        words.extend(4..256i16);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut instructions = vec![Instruction {
            opcode: 4097,
            on_true: 254,
            on_false: 255,
            operands: if i["default_args"] == true {
                [255; 8]
            } else {
                bytes[..8].try_into().unwrap()
            },
        }];
        for b in bytes[8..].chunks(12) {
            let mut raw = [0; 12];
            raw[..b.len()].copy_from_slice(b);
            instructions.push(Instruction {
                opcode: u16::from_le_bytes([raw[0], raw[1]]),
                on_true: raw[2],
                on_false: raw[3],
                operands: raw[4..].try_into().unwrap(),
            });
        }
        parent.body.instructions = instructions;
        p.modules.get_mut("fixture").unwrap().routines.insert(
            4097,
            routine(
                i["format"].as_u64().unwrap() as u16,
                i["kind"].as_u64().unwrap() as u8,
                i["argc"].as_u64().unwrap() as u8,
                i["locals"].as_u64().unwrap() as u16,
                vec![ins(); 3],
            ),
        );
        let t = vm.threads.get_mut(&1).unwrap();
        t.stack[0].args.clear();
        t.stack[0].locals.clear();
        t.frame_memory = vec![Some(-23131); 1000];
        vm.run_slice(&p, 1, 1).unwrap();
        let t = &vm.threads[&1];
        let actual = if t.stack.len() == 1 {
            assert_eq!(i["kind"], 4);
            assert!(i["entry"].as_i64().unwrap() < 0 && i["default_args"] == false);
            assert!(
                matches!(&t.status,Status::Faulted(f) if f.message=="negative indexed BHAV entry")
            );
            json!({"started":false,"error":null})
        } else {
            assert!(!matches!(t.status, Status::Faulted(_)));
            let f = t.stack.last().unwrap();
            json!({"error":null,"started":true,"pc":f.location.instruction,"stack_object":f.stack_object,"argc":f.args.len(),"locals":f.locals.len(),"args":f.args,"local_values":f.locals})
        };
        count += 1;
        if actual != row["original"] {
            failures += 1;
            if failures <= 20 {
                eprintln!("frame {i}: Rust {actual}; original {}", row["original"]);
            }
        }
    }
    println!("{count} memory/frame cases compared; {failures} mismatches");
    if failures != 0 {
        std::process::exit(1);
    }
}
