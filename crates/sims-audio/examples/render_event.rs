//! Renders sound events to a WAV file: `render_event <out.wav> <event>...`.
//!
//! Reads the install at `SIMS_INSTALL`. Each event starts one second after
//! the one before, for object 1, with `SimSpeed` at 1; rendering stops
//! when everything is quiet or after a minute. Write the WAV outside the
//! repository: it is made from game data.

use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use sims_audio::player::{NoSources, Player, RATE};
use sims_audio::{Pcm, SoundEvent};

fn main() {
    let root = std::env::var_os("SIMS_INSTALL").expect("set SIMS_INSTALL to the install");
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .expect("usage: render_event <out.wav> <event>...");
    let events: Vec<String> = args.collect();
    let files = SoundFiles::scan(std::path::Path::new(&root)).expect("scan the install");
    let modules = Module::load_all(&files).expect("load the sound modules");
    let mut player = Player::new(modules, files, 1);
    player.set_global(100, 1);
    let mut samples = Vec::new();
    for name in &events {
        player.handle(
            &SoundEvent::Play {
                object: 1,
                name: name.clone(),
            },
            &NoSources,
        );
        samples.extend(player.render(RATE as usize, &NoSources));
    }
    samples.extend(player.render_until_idle(60 * RATE as usize, &NoSources));
    for fault in &player.faults {
        eprintln!("fault: {fault}");
    }
    let pcm = Pcm {
        rate: RATE,
        channels: 1,
        samples,
    };
    std::fs::write(&out, pcm.to_wav().expect("make the WAV file")).expect("write the WAV file");
    eprintln!("{out}: {} frames", pcm.frames());
}
