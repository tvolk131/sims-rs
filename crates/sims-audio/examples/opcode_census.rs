//! Counts each instruction across the install's HIT programs, then how
//! often each register is named, by instruction: `opcode_census`. Reads
//! the install at `SIMS_INSTALL`; prints counts only.

use sims_audio::files::SoundFiles;
use sims_audio::hit::Operand;
use sims_audio::module::Module;
use std::collections::BTreeMap;

fn main() {
    let root = std::env::var_os("SIMS_INSTALL").expect("set SIMS_INSTALL");
    let files = SoundFiles::scan(std::path::Path::new(&root)).expect("scan the install");
    let modules = Module::load_all(&files).expect("load the sound modules");
    let mut counts: BTreeMap<(u8, &str), usize> = BTreeMap::new();
    let mut registers: BTreeMap<i64, BTreeMap<&str, usize>> = BTreeMap::new();
    for module in &modules {
        let code = module
            .program
            .disassemble(&|id| module.is_target(id))
            .expect("disassemble");
        for instruction in code {
            *counts
                .entry((instruction.opcode, instruction.name()))
                .or_default() += 1;
            for &(kind, value) in &instruction.operands {
                if kind == Operand::Register {
                    *registers
                        .entry(value)
                        .or_default()
                        .entry(instruction.name())
                        .or_default() += 1;
                }
            }
        }
    }
    for ((opcode, name), count) in counts {
        println!("{opcode:3} {name:16} {count}");
    }
    for (register, uses) in registers {
        println!("r{register:<4} {uses:?}");
    }
}
