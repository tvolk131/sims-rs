//! Prints a HIT module's code from an address: `hit_dump <module> <address> [count]`.
//! Reads the install at `SIMS_INSTALL`; prints to the terminal only.

use sims_audio::files::SoundFiles;
use sims_audio::module::Module;

fn main() {
    let root = std::env::var_os("SIMS_INSTALL").expect("set SIMS_INSTALL");
    let mut args = std::env::args().skip(1);
    let name = args
        .next()
        .expect("module name, such as sounddata\\simssound");
    let address = args.next().expect("address");
    let address = u32::from_str_radix(address.trim_start_matches("0x"), 16).expect("hex address");
    let count: usize = args.next().map_or(16, |c| c.parse().expect("count"));
    let files = SoundFiles::scan(std::path::Path::new(&root)).unwrap();
    let modules = Module::load_all(&files).unwrap();
    let module = modules
        .iter()
        .find(|m| m.name == name)
        .expect("no such module");
    let labels: std::collections::BTreeMap<i64, &str> = module
        .symbols
        .lines
        .windows(2)
        .filter_map(|w| match w {
            [(tkd, _), (label, address)] if tkd.strip_prefix("tkd_") == Some(label) => {
                Some((*address, label.as_str()))
            }
            _ => None,
        })
        .collect();
    let mut at = address;
    for _ in 0..count {
        let Ok(i) = module.program.decode(at, &|id| module.is_target(id)) else {
            break;
        };
        if let Some(label) = labels.get(&i64::from(at)) {
            println!("{label}:");
        }
        println!("  {at:#06x} {} {:?}", i.name(), i.values());
        at += i.length;
    }
}
