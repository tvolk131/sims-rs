//! Draws the live-mode control panel for a saved house and writes PNGs.
//!
//! cargo run -p sims-ui --example hud -- <out dir> [house] [width height]...
//!
//! `SIMS_INSTALL` names the install; the house defaults to the Goths'.
//! Without sizes it draws 800 by 600 and 1280 by 720. Each size is drawn
//! with the strip closed (as after loading) and open (after pressing
//! Live), on black.

#[path = "../tests/common/mod.rs"]
mod common;

use sims_ui::frame::Frame;
use sims_ui::hud::{self, Controls};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out) = args.first() else {
        eprintln!("usage: hud <out dir> [house] [width height]...");
        std::process::exit(2);
    };
    let Some(root) = common::install_root() else {
        eprintln!("set SIMS_INSTALL to the install folder");
        std::process::exit(2);
    };
    let house = args.get(1).map_or(common::GOTH_HOUSE, String::as_str);
    let mut sizes: Vec<[i32; 2]> = args[2.min(args.len())..]
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|[w, h]| Some([w.parse().ok()?, h.parse().ok()?]))
        .collect();
    if sizes.is_empty() {
        sizes = vec![[800, 600], [1280, 720]];
    }
    let opened = common::open(&root, house);
    for size in sizes {
        for expanded in [false, true] {
            let controls = Controls {
                expanded,
                ..Controls::default()
            };
            let mut frame = Frame::new(size[0] as u32, size[1] as u32, [0, 0, 0]);
            let items = hud::layout(size, &opened.snapshot, &controls);
            hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
            let name = format!(
                "{out}/hud-{}x{}-{}.png",
                size[0],
                size[1],
                if expanded { "open" } else { "closed" }
            );
            std::fs::write(&name, frame.png()).unwrap();
            println!("{name}");
        }
    }
}
