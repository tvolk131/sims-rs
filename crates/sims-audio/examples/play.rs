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
//! The player renders 22,050 Hz stereo on this thread into a ring of
//! frames allocated once; the device's callback takes frames from it,
//! converting them to the device's rate and format, and allocates
//! nothing. Nothing here is part of the simulation: the device paces the
//! rendering. If the device stops taking frames for two seconds, or
//! reports that the stream is gone, the program stops with an error.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig};
use sims_audio::SoundEvent;
use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use sims_audio::player::{BLOCK, NoSources, Player, RATE};
use sims_audio::stations::StationLists;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Frames rendered ahead of the device: about 0.2 s.
const CAPACITY: usize = 64 * BLOCK;
/// How long the main thread waits for the device at a time, and how many
/// waits in a row with no callback count as the device having stopped.
const WAIT: Duration = Duration::from_millis(100);
const STALLS: u32 = 20;
/// 1.0 in the converter's 16.16 fixed point.
const ONE: u64 = 1 << 16;

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

/// What the main thread and the device's callback share.
struct Shared {
    ring: Mutex<Ring>,
    /// Signalled after each callback.
    moved: Condvar,
    /// Set when the stream reports that it cannot go on.
    failed: AtomicBool,
}

struct Ring {
    frames: VecDeque<[i16; 2]>,
    /// No more frames will come.
    ended: bool,
    /// Ended, and the last frame has had time to reach the speakers.
    finished: bool,
    callbacks: u64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits until `done` holds, failing if the stream failed or stopped
    /// calling back.
    fn wait_until(&self, done: impl Fn(&Ring) -> bool) -> Result<MutexGuard<'_, Ring>, String> {
        let mut ring = self.lock();
        let mut stalls = 0;
        while !done(&ring) {
            if self.failed.load(Ordering::Relaxed) {
                return Err("the output stream failed".into());
            }
            let before = ring.callbacks;
            ring = self
                .moved
                .wait_timeout(ring, WAIT)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            stalls = if ring.callbacks == before {
                stalls + 1
            } else {
                0
            };
            if stalls >= STALLS {
                return Err("the output device stopped taking sound".into());
            }
        }
        Ok(ring)
    }
}

/// Converts 22,050 Hz frames to the device's rate by linear
/// interpolation.
struct Converter {
    /// Source frames per device frame, 16.16.
    step: u64,
    /// Position between the first two frames, 16.16; at or past
    /// [`ONE`], frames still owed a skip.
    phase: u64,
}

impl Converter {
    fn new(rate: u32) -> Self {
        Converter {
            step: (u64::from(RATE) << 16) / u64::from(rate.max(1)),
            phase: 0,
        }
    }

    /// The next device frame, or `None` when the frames have run out (or,
    /// once `ended`, are over).
    fn next(&mut self, frames: &mut VecDeque<[i16; 2]>, ended: bool) -> Option<[f32; 2]> {
        while self.phase >= ONE && !frames.is_empty() {
            frames.pop_front();
            self.phase -= ONE;
        }
        if self.phase >= ONE {
            return None;
        }
        let a = *frames.front()?;
        let b = match frames.get(1) {
            Some(b) => *b,
            None if ended => a,
            None => return None,
        };
        let t = self.phase as f32 / ONE as f32;
        let mix = |x: i16, y: i16| (f32::from(x) * (1.0 - t) + f32::from(y) * t) / 32768.0;
        self.phase += self.step;
        Some([mix(a[0], b[0]), mix(a[1], b[1])])
    }
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
    let shared = Arc::new(Shared {
        ring: Mutex::new(Ring {
            frames: VecDeque::with_capacity(CAPACITY),
            ended: false,
            finished: false,
            callbacks: 0,
        }),
        moved: Condvar::new(),
        failed: AtomicBool::new(false),
    });
    let stream = match format {
        SampleFormat::I16 => stream::<i16>(&device, config, shared.clone()),
        SampleFormat::I32 => stream::<i32>(&device, config, shared.clone()),
        SampleFormat::F32 => stream::<f32>(&device, config, shared.clone()),
        SampleFormat::F64 => stream::<f64>(&device, config, shared.clone()),
        format => return Err(format!("sample format {format} is not supported")),
    }?;

    for name in &options.events {
        player.handle(&SoundEvent::Play {
            object: 1,
            name: name.clone(),
        });
    }
    let blocks = options.seconds.saturating_mul(u64::from(RATE)) / BLOCK as u64;
    let mut playing = false;
    for _ in 0..blocks {
        if let Some(view) = options.view {
            player.set_ambience_view(view, options.hour.unwrap_or(u32::MAX));
        }
        let block = player.render_stereo(BLOCK, &NoSources);
        // The device starts once the ring is full, so it does not run dry
        // at once.
        if !playing && shared.lock().frames.len() + BLOCK > CAPACITY {
            stream.play().map_err(|e| format!("play: {e}"))?;
            playing = true;
        }
        let mut ring = shared.wait_until(|ring| ring.frames.len() + BLOCK <= CAPACITY)?;
        ring.frames.extend(block.as_chunks::<2>().0.iter().copied());
        drop(ring);
        if player.is_idle() {
            break;
        }
    }
    shared.lock().ended = true;
    if !playing {
        stream.play().map_err(|e| format!("play: {e}"))?;
    }
    drop(shared.wait_until(|ring| ring.finished)?);
    drop(stream);
    for fault in &player.faults {
        eprintln!("fault: {fault}");
    }
    Ok(())
}

/// The device's stream, taking frames from the ring.
fn stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let rate = config.sample_rate.max(1);
    let mut converter = Converter::new(rate);
    // Device frames of silence written since the frames ended.
    let mut tail: u64 = 0;
    let errors = shared.clone();
    let callback = move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
        let mut ring = shared.lock();
        let ended = ring.ended;
        let mut silent: u64 = 0;
        for frame in out.chunks_mut(channels) {
            let [left, right] = converter.next(&mut ring.frames, ended).unwrap_or_else(|| {
                silent += 1;
                [0.0, 0.0]
            });
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
        if ended && ring.frames.is_empty() {
            tail += silent;
            // Done once the silence after the end covers the device's
            // latency and a tenth of a second more.
            let timestamp = info.timestamp();
            let latency = timestamp.playback.duration_since(timestamp.callback);
            let latency =
                u64::try_from(latency.as_micros() * u128::from(rate) / 1_000_000).unwrap_or(0);
            if tail >= latency + u64::from(rate) / 10 {
                ring.finished = true;
            }
        }
        ring.callbacks += 1;
        drop(ring);
        shared.moved.notify_one();
    };
    let on_error = move |error: cpal::Error| {
        eprintln!("stream: {error}");
        if !matches!(
            error.kind(),
            ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied
        ) {
            errors.failed.store(true, Ordering::Relaxed);
            errors.moved.notify_one();
        }
    };
    device
        .build_output_stream(config, callback, on_error, None)
        .map_err(|e| format!("stream: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts `frames` source frames alternating between ±30,000 to
    /// `rate`, adding them in uneven batches, and checks every output lies
    /// within the input's range. Returns the device frames made.
    fn convert(rate: u32, frames: usize) -> usize {
        let mut converter = Converter::new(rate);
        let mut ring = VecDeque::new();
        let mut fed = 0;
        let mut out = 0;
        let mut batch = 1;
        loop {
            let ended = fed == frames;
            match converter.next(&mut ring, ended) {
                Some([l, r]) => {
                    let bound = 30_000.0 / 32_768.0 + 1e-6;
                    assert!(l.abs() <= bound && (l - r).abs() < 1e-6, "{rate}: {l} {r}");
                    out += 1;
                }
                None if ended => return out,
                None => {
                    for _ in 0..batch.min(frames - fed) {
                        let v = if fed % 2 == 0 { 30_000 } else { -30_000 };
                        ring.push_back([v, v]);
                        fed += 1;
                    }
                    batch = batch % 5 + 1;
                }
            }
        }
    }

    #[test]
    fn the_converter_stays_in_range_at_any_rate() {
        for rate in [8_000, 11_025, 22_050, 44_100, 48_000, 96_000] {
            let out = convert(rate, 22_050);
            let expected = 22_050 * u64::from(rate) / u64::from(RATE);
            // The 16.16 step rounds down: at most a frame in 10,000 more.
            assert!(
                out.abs_diff(expected as usize) <= expected as usize / 10_000 + 2,
                "{rate}: {out} {expected}"
            );
        }
    }
}
