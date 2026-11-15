//! Plays the [`Mixer`] on the default output device.
//!
//! A thread owns the mixer: it takes [`Input`]s from the game, renders
//! 22,050 Hz stereo blocks into a ring about a fifth of a second long,
//! and waits while the ring is full. The device's callback takes frames
//! from the ring, converting them to the device's rate and format by
//! linear interpolation, and allocates nothing. When the ring runs dry
//! the device plays silence; the game never waits on sound.

use crate::sound::{Input, Mixer};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use sims_audio::player::{BLOCK, RATE};
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Frames rendered ahead of the device: about 0.2 s.
const CAPACITY: usize = 64 * BLOCK;

/// 1.0 in the converter's 16.16 fixed point.
const ONE: u64 = 1 << 16;

/// The longest the mixer thread waits for room before it looks for input
/// again.
const WAIT: Duration = Duration::from_millis(20);

struct Shared {
    frames: Mutex<VecDeque<[i16; 2]>>,
    /// Signalled when the device takes frames.
    taken: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, VecDeque<[i16; 2]>> {
        self.frames.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Sound playing on the device. Dropping it stops the sound.
pub struct Output {
    inputs: Sender<Input>,
    _stream: cpal::Stream,
}

impl Output {
    /// Starts the device and the mixer thread.
    pub fn start(mixer: Mixer) -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no sound output device")?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("sound output: {e}"))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let shared = Arc::new(Shared {
            frames: Mutex::new(VecDeque::with_capacity(CAPACITY)),
            taken: Condvar::new(),
        });
        let stream = match format {
            SampleFormat::I16 => stream::<i16>(&device, config, shared.clone()),
            SampleFormat::I32 => stream::<i32>(&device, config, shared.clone()),
            SampleFormat::F32 => stream::<f32>(&device, config, shared.clone()),
            SampleFormat::F64 => stream::<f64>(&device, config, shared.clone()),
            format => return Err(format!("sound sample format {format} is not supported")),
        }?;
        let (inputs, received) = mpsc::channel();
        std::thread::Builder::new()
            .name("sound".into())
            .spawn(move || mix(mixer, &received, &shared))
            .map_err(|e| format!("sound thread: {e}"))?;
        stream.play().map_err(|e| format!("sound output: {e}"))?;
        Ok(Self {
            inputs,
            _stream: stream,
        })
    }

    /// Sends the mixer an input. Sound that has stopped is not an error
    /// the game should stop for, so a closed thread is ignored.
    pub fn send(&self, input: Input) {
        let _ = self.inputs.send(input);
    }
}

/// The mixer thread: inputs in, blocks out, until the game drops its end.
fn mix(mut mixer: Mixer, inputs: &Receiver<Input>, shared: &Shared) {
    loop {
        loop {
            match inputs.try_recv() {
                Ok(input) => mixer.input(input),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for fault in mixer.new_faults() {
            eprintln!("sims-game: sound: {fault}");
        }
        {
            let frames = shared.lock();
            if frames.len() + BLOCK > CAPACITY {
                drop(
                    shared
                        .taken
                        .wait_timeout(frames, WAIT)
                        .unwrap_or_else(PoisonError::into_inner),
                );
                continue;
            }
        }
        let block = mixer.render(BLOCK);
        shared
            .lock()
            .extend(block.as_chunks::<2>().0.iter().copied());
    }
}

/// Converts 22,050 Hz frames to the device's rate by linear
/// interpolation.
struct Converter {
    /// Source frames per device frame, 16.16.
    step: u64,
    /// Position between the first two frames, 16.16.
    phase: u64,
}

impl Converter {
    fn new(rate: u32) -> Self {
        Self {
            step: (u64::from(RATE) << 16) / u64::from(rate.max(1)),
            phase: 0,
        }
    }

    /// The next device frame, or None when the frames have run out.
    fn next(&mut self, frames: &mut VecDeque<[i16; 2]>) -> Option<[f32; 2]> {
        while self.phase >= ONE && frames.len() > 1 {
            frames.pop_front();
            self.phase -= ONE;
        }
        if self.phase >= ONE {
            return None;
        }
        let (a, b) = (*frames.front()?, *frames.get(1)?);
        let t = self.phase as f32 / ONE as f32;
        let mix = |x: i16, y: i16| (f32::from(x) * (1.0 - t) + f32::from(y) * t) / 32768.0;
        self.phase += self.step;
        Some([mix(a[0], b[0]), mix(a[1], b[1])])
    }
}

fn stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let mut converter = Converter::new(config.sample_rate);
    let callback = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
        let mut frames = shared.lock();
        for frame in out.chunks_mut(channels) {
            let [left, right] = converter.next(&mut frames).unwrap_or([0.0, 0.0]);
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
        drop(frames);
        shared.taken.notify_one();
    };
    let on_error = |error: cpal::Error| eprintln!("sims-game: sound: {error}");
    device
        .build_output_stream(config, callback, on_error, None)
        .map_err(|e| format!("sound output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_converter_stays_in_range_and_keeps_the_rate() {
        for rate in [11_025, 22_050, 44_100, 48_000] {
            let mut converter = Converter::new(rate);
            let mut frames: VecDeque<[i16; 2]> = (0..22_050)
                .map(|i| {
                    if i % 2 == 0 {
                        [30_000; 2]
                    } else {
                        [-30_000; 2]
                    }
                })
                .collect();
            let mut out = 0usize;
            while let Some([l, r]) = converter.next(&mut frames) {
                assert!(l.abs() <= 30_000.0 / 32_768.0 + 1e-6 && (l - r).abs() < 1e-6);
                out += 1;
            }
            let expected = 22_050 * rate as usize / RATE as usize;
            assert!(
                out.abs_diff(expected) <= expected / 1000 + 2,
                "{rate}: {out}"
            );
        }
    }
}
