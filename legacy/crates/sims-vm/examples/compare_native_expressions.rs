//! Compare our VM with a private report produced by scripts/research/native_oracle.py.
//! Original executable bytes are neither embedded nor required by ordinary tests.
use serde::Deserialize;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Object, Program, Routine, Start, Status, Stop, Vm, World};
use std::{collections::BTreeMap, fs};

#[derive(Deserialize, Debug)]
struct Input {
    operator: u8,
    lhs: i16,
    rhs: i16,
    lhs_scope: Option<u8>,
    rhs_scope: Option<u8>,
    lhs_index: Option<i16>,
    rhs_index: Option<i16>,
    flags: Option<u8>,
}
#[derive(Deserialize, Debug, PartialEq)]
struct Outcome {
    error: Option<u32>,
    returned: Option<bool>,
    value: i16,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Outcome,
}
#[derive(Deserialize)]
struct Report {
    schema: u32,
    capture_sha256: String,
    cases: Vec<Case>,
}
fn set(vm: &mut Vm, scope: u8, index: i16, value: i16) {
    match scope {
        0 => vm.world.objects.get_mut(&1).unwrap().attributes[index as usize] = value,
        3 => {
            vm.world
                .objects
                .get_mut(&1)
                .unwrap()
                .data
                .insert(index, value);
        }
        8 => vm.world.objects.get_mut(&1).unwrap().temps[index as usize] = value,
        7 => (),
        _ => panic!("unhandled fixture scope {scope}"),
    }
}
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("supply the private original-expression report");
    let report: Report = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(report.schema, 1);
    assert_eq!(
        report.capture_sha256,
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert!(!report.cases.is_empty());
    let mut failures = 0;
    for case in &report.cases {
        let i = &case.input;
        let ls = i.lhs_scope.unwrap_or(8);
        let rs = i.rhs_scope.unwrap_or(7);
        let li = i.lhs_index.unwrap_or(0);
        let ri = i.rhs_index.unwrap_or(if rs == 7 { i.rhs } else { 1 });
        let mut operands = [0; 8];
        operands[..2].copy_from_slice(&li.to_le_bytes());
        operands[2..4].copy_from_slice(&ri.to_le_bytes());
        operands[4..].copy_from_slice(&[i.flags.unwrap_or(0), i.operator, ls, rs]);
        let program = Program {
            globals: "fixture".into(),
            modules: BTreeMap::from([(
                "fixture".into(),
                Module {
                    source: "fixture".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "synthetic oracle case".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 0,
                                locals: 0,
                                instructions: vec![Instruction {
                                    opcode: 2,
                                    on_true: 254,
                                    on_false: 255,
                                    operands,
                                }],
                            },
                        },
                    )]),
                    ..Module::default()
                },
            )]),
            ..Program::default()
        };
        let mut vm = Vm::new(
            World {
                objects: BTreeMap::from([(
                    1,
                    Object {
                        attributes: vec![0; 8],
                        ..Object::default()
                    },
                )]),
                ..World::default()
            },
            1,
        );
        vm.spawn(
            &program,
            Start {
                owner: "fixture",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
        set(&mut vm, ls, li, i.lhs);
        if ls != rs || li != ri {
            set(&mut vm, rs, ri, i.rhs);
        }
        let run = vm.run_slice(&program, 1, 1).unwrap();
        let value = match ls {
            0 => vm.world.objects[&1].attributes[li as usize],
            3 => vm.world.objects[&1].data[&li],
            8 => vm.world.objects[&1].temps[li as usize],
            7 => i.lhs,
            _ => unreachable!(),
        };
        let error = match &vm.threads[&1].status {
            Status::Faulted(f) => {
                let code = f
                    .message
                    .split("native error 0x")
                    .nth(1)
                    .and_then(|s| s.get(..2))
                    .and_then(|s| u32::from_str_radix(s, 16).ok());
                Some(code.unwrap_or(u32::MAX))
            }
            _ => None,
        };
        let outcome = Outcome {
            error,
            value,
            returned: match run.stop {
                Stop::Returned(b) => Some(b),
                _ => None,
            },
        };
        if outcome != case.original {
            failures += 1;
            if failures <= 20 {
                eprintln!("{i:?}: original {:?}; Rust {outcome:?}", case.original);
            }
        }
    }
    println!(
        "{} cases compared; {failures} mismatches",
        report.cases.len()
    );
    if failures != 0 {
        std::process::exit(1);
    }
}
