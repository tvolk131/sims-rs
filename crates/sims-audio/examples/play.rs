//! Plays sound events live on the default output device:
//! `play [--seconds N] [--view V --hour H] <event>...`.
//!
//! Needs the `playback` feature (`cargo run -p sims-audio --features
//! playback --example play -- ...`), which nothing else uses, so tests
//! and CI need no audio device. Reads the install at `SIMS_INSTALL`,
//! with its stations and ambient scores. The events start at once, in
//! order, for object 1; playing stops when everything is quiet or after
//! `--seconds` (default 60). `--view` and `--hour` move the ambience's
//! cursor as the game's camera and clock would (see
//! `Player::set_ambience_view`). For the backyard ambience at noon:
//!
//! ```text
//! play --view 50 --hour 12 bkground_nhood1 setambience_backyard bkground_live
//! ```
//!
//! The player renders 22,050 Hz stereo on this thread; the device's
//! callback converts it to the device's rate and format. Nothing here is
//! part of the simulation: wall-clock pacing comes from the device.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use sims_audio::SoundEvent;
use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use sims_audio::player::{BLOCK, NoSources, Player, RATE};
use sims_audio::stations::StationLists;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

/// Blocks rendered ahead of the device: about 0.2 s.
const AHEAD: usize = 64;

struct Options {
    seconds: u64,
    view: Option<i32>,
    hour: Option<u32>,
    events: Vec<String>,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        seconds: 60,
        view: None,
        hour: None,
        events: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--seconds" => {
                options.seconds = value("--seconds")?
                    .parse()
                    .map_err(|e| format!("--seconds: {e}"))?;
            }
            "--view" => {
                options.view = Some(
                    value("--view")?
                        .parse()
                        .map_err(|e| format!("--view: {e}"))?,
                );
            }
            "--hour" => {
                options.hour = Some(
                    value("--hour")?
                        .parse()
                        .map_err(|e| format!("--hour: {e}"))?,
                );
            }
            _ => options.events.push(arg),
        }
    }
    if options.events.is_empty() {
        return Err("usage: play [--seconds N] [--view V --hour H] <event>...".into());
    }
    Ok(options)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = options()?;
    let root = std::env::var_os("SIMS_INSTALL").ok_or("set SIMS_INSTALL to the install")?;
    let files = SoundFiles::scan(std::path::Path::new(&root)).map_err(|e| format!("scan: {e}"))?;
    let modules = Module::load_all(&files).map_err(|e| format!("modules: {e}"))?;
    let lists = StationLists::build(&files);
    let mut player = Player::new(modules, files.clone(), 1);
    player.add_stations(&lists);
    player.add_scores(&files);
    player.set_global(100, 1);

    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no output device")?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("output config: {e}"))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    eprintln!(
        "playing at {} Hz, {} channels, {format}",
        config.sample_rate, config.channels
    );
    let (blocks, from_player) = sync_channel::<Vec<i16>>(AHEAD);
    let (drained, done) = sync_channel::<()>(1);
    let stream = match format {
        SampleFormat::I16 => stream::<i16>(&device, config, from_player, drained),
        SampleFormat::I32 => stream::<i32>(&device, config, from_player, drained),
        SampleFormat::F32 => stream::<f32>(&device, config, from_player, drained),
        SampleFormat::F64 => stream::<f64>(&device, config, from_player, drained),
        format => return Err(format!("sample format {format} is not supported")),
    }?;
    for name in &options.events {
        player.handle(&SoundEvent::Play {
            object: 1,
            name: name.clone(),
        });
    }
    let blocks_total = options.seconds.saturating_mul(u64::from(RATE)) / BLOCK as u64;
    let mut playing = false;
    for sent in 0..blocks_total {
        if let Some(view) = options.view {
            player.set_ambience_view(view, options.hour.unwrap_or(u32::MAX));
        }
        if send(&blocks, player.render_stereo(BLOCK, &NoSources)).is_err() || player.is_idle() {
            break;
        }
        // The device starts once the channel is full, so it does not
        // run dry at once.
        if !playing && sent + 1 >= AHEAD as u64 {
            stream.play().map_err(|e| format!("play: {e}"))?;
            playing = true;
        }
    }
    drop(blocks);
    if !playing {
        stream.play().map_err(|e| format!("play: {e}"))?;
    }
    // The callback says when it has played what was sent.
    let _ = done.recv();
    for fault in &player.faults {
        eprintln!("fault: {fault}");
    }
    Ok(())
}

fn send(blocks: &SyncSender<Vec<i16>>, block: Vec<i16>) -> Result<(), ()> {
    blocks.send(block).map_err(|_| ())
}

/// The device's stream, pulling the player's blocks and converting them
/// by linear interpolation.
fn stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    blocks: Receiver<Vec<i16>>,
    drained: SyncSender<()>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    // Source frames per device frame, 16.16 fixed point.
    let step = (u64::from(RATE) << 16) / u64::from(config.sample_rate.max(1));
    let mut frames: VecDeque<[i16; 2]> = VecDeque::new();
    let mut phase: u64 = 0;
    let mut ended = false;
    let callback = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
        for frame in out.chunks_mut(channels) {
            while frames.len() < 2 && !ended {
                match blocks.try_recv() {
                    Ok(block) => frames.extend(block.chunks(2).map(|p| [p[0], p[1]])),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => ended = true,
                }
            }
            let (left, right) = match (frames.front(), frames.get(1)) {
                (Some(a), Some(b)) => {
                    let mix = |x: i16, y: i16| {
                        let t = phase as f32 / 65536.0;
                        (f32::from(x) * (1.0 - t) + f32::from(y) * t) / 32768.0
                    };
                    (mix(a[0], b[0]), mix(a[1], b[1]))
                }
                (Some(a), None) if ended => (f32::from(a[0]) / 32768.0, f32::from(a[1]) / 32768.0),
                _ => (0.0, 0.0),
            };
            if frames.len() >= 2 || (ended && !frames.is_empty()) {
                phase += step;
                while phase >= 1 << 16 && !frames.is_empty() {
                    phase -= 1 << 16;
                    frames.pop_front();
                }
            }
            if ended && frames.is_empty() {
                let _ = drained.try_send(());
            }
            match frame {
                [only] => *only = T::from_sample((left + right) / 2.0),
                [l, r, rest @ ..] => {
                    *l = T::from_sample(left);
                    *r = T::from_sample(right);
                    for sample in rest {
                        *sample = T::from_sample(0.0);
                    }
                }
                [] => {}
            }
        }
    };
    device
        .build_output_stream(config, callback, |error| eprintln!("stream: {error}"), None)
        .map_err(|e| format!("stream: {e}"))
}
