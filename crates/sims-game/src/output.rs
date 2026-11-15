//! Plays the [`Mixer`] on the default output device.
//!
//! A thread owns the mixer: it takes [`Input`]s from the game, renders
//! 22,050 Hz stereo blocks into a ring about a fifth of a second long,
//! and sleeps briefly while the ring is full. The device's callback takes
//! frames from the ring, converting them to the device's rate and format
//! by linear interpolation. The ring is a single-producer,
//! single-consumer queue of atomics, so the callback never takes a lock,
//! never waits on the mixer and allocates nothing. When the ring runs dry
//! the device plays silence; the game never waits on sound. When the
//! stream fails for good, the mixer thread ends.

use crate::sound::{Input, Mixer};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig};
use sims_audio::player::{BLOCK, RATE};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

/// Frames rendered ahead of the device: about 0.2 s.
const CAPACITY: usize = 64 * BLOCK;

/// 1.0 in the converter's 16.16 fixed point.
const ONE: u64 = 1 << 16;

/// How long the mixer thread sleeps while the ring is full: a small part
/// of what the ring holds, so the device never runs it dry for this.
const NAP: Duration = Duration::from_millis(5);

/// The longest the device waits for the mixer before it starts.
const PREFILL: Duration = Duration::from_millis(500);

/// A ring of stereo frames for one producer and one consumer. The read
/// and write counts only grow (wrapping), and a frame's slot is its count
/// modulo the capacity. Each side writes only its own count, publishing
/// it with release ordering after the slots it covers.
struct Ring {
    slots: Box<[AtomicU32]>,
    read: AtomicUsize,
    write: AtomicUsize,
}

fn pack([l, r]: [i16; 2]) -> u32 {
    u32::from(l as u16) | (u32::from(r as u16) << 16)
}

fn unpack(word: u32) -> [i16; 2] {
    [word as u16 as i16, (word >> 16) as u16 as i16]
}

impl Ring {
    fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity.max(1)).map(|_| AtomicU32::new(0)).collect(),
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        }
    }

    fn len(&self) -> usize {
        let write = self.write.load(Ordering::Acquire);
        write.wrapping_sub(self.read.load(Ordering::Acquire))
    }

    /// The producer adds `frames`, all or none. False if they do not fit.
    fn push(&self, frames: &[[i16; 2]]) -> bool {
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        if write.wrapping_sub(read) + frames.len() > self.slots.len() {
            return false;
        }
        for (i, &frame) in frames.iter().enumerate() {
            let slot = write.wrapping_add(i) % self.slots.len();
            self.slots[slot].store(pack(frame), Ordering::Relaxed);
        }
        self.write
            .store(write.wrapping_add(frames.len()), Ordering::Release);
        true
    }

    /// The consumer's `n`th frame from the front, if there is one.
    fn peek(&self, n: usize) -> Option<[i16; 2]> {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        if write.wrapping_sub(read) <= n {
            return None;
        }
        let slot = read.wrapping_add(n) % self.slots.len();
        Some(unpack(self.slots[slot].load(Ordering::Relaxed)))
    }

    /// The consumer drops the front frame, if there is one.
    fn pop(&self) {
        let read = self.read.load(Ordering::Relaxed);
        if self.write.load(Ordering::Acquire) != read {
            self.read.store(read.wrapping_add(1), Ordering::Release);
        }
    }
}

struct Shared {
    ring: Ring,
    /// Set when the stream has failed for good.
    failed: AtomicBool,
    /// Set once an underrun is reported, so later ones are not.
    underrun: AtomicBool,
}

/// Sound playing on the device. Dropping it stops the sound, and the
/// mixer thread ends at its next look for input.
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
            ring: Ring::new(CAPACITY),
            failed: AtomicBool::new(false),
            underrun: AtomicBool::new(false),
        });
        let stream = match format {
            SampleFormat::I16 => stream::<i16>(&device, config, shared.clone()),
            SampleFormat::I32 => stream::<i32>(&device, config, shared.clone()),
            SampleFormat::F32 => stream::<f32>(&device, config, shared.clone()),
            SampleFormat::F64 => stream::<f64>(&device, config, shared.clone()),
            format => return Err(format!("sound sample format {format} is not supported")),
        }?;
        let ready = Arc::clone(&shared);
        let (inputs, received) = mpsc::channel();
        std::thread::Builder::new()
            .name("sound".into())
            .spawn(move || mix(mixer, &received, &shared))
            .map_err(|e| format!("sound thread: {e}"))?;
        // Half a ring ahead before the device starts, so it does not
        // start dry; a mixer too slow for that starts it anyway.
        let mut waited = Duration::ZERO;
        while ready.ring.len() < CAPACITY / 2 && waited < PREFILL {
            std::thread::sleep(NAP);
            waited += NAP;
        }
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

/// The mixer thread: inputs in, blocks out, until the game drops its end
/// or the stream fails for good.
fn mix(mut mixer: Mixer, inputs: &Receiver<Input>, shared: &Shared) {
    loop {
        loop {
            match inputs.try_recv() {
                Ok(input) => mixer.input(input),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if shared.failed.load(Ordering::Relaxed) {
            return;
        }
        for fault in mixer.new_faults() {
            eprintln!("sims-game: sound: {fault}");
        }
        if shared.ring.len() + BLOCK > CAPACITY {
            std::thread::sleep(NAP);
            continue;
        }
        let block = mixer.render(BLOCK);
        // Only this thread adds frames, so the room checked is still there.
        shared.ring.push(block.as_chunks::<2>().0);
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
    fn next(&mut self, ring: &Ring) -> Option<[f32; 2]> {
        while self.phase >= ONE && ring.peek(1).is_some() {
            ring.pop();
            self.phase -= ONE;
        }
        if self.phase >= ONE {
            return None;
        }
        let (a, b) = (ring.peek(0)?, ring.peek(1)?);
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
    let failed = Arc::clone(&shared);
    let callback = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
        for frame in out.chunks_mut(channels) {
            let [left, right] = converter.next(&shared.ring).unwrap_or([0.0, 0.0]);
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
    let on_error = move |error: cpal::Error| {
        let underrun = error.kind() == ErrorKind::Xrun;
        if !underrun || !failed.underrun.swap(true, Ordering::Relaxed) {
            eprintln!("sims-game: sound: {error}");
        }
        // An underrun, a device change or a denied priority leaves the
        // stream running; anything else ends it.
        if !matches!(
            error.kind(),
            ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied
        ) {
            failed.failed.store(true, Ordering::Relaxed);
        }
    };
    device
        .build_output_stream(config, callback, on_error, None)
        .map_err(|e| format!("sound output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_wraps_and_refuses_what_does_not_fit() {
        let ring = Ring::new(4);
        assert!(ring.push(&[[1, -1], [2, -2], [3, -3]]));
        assert!(!ring.push(&[[4, 4], [5, 5]]));
        assert_eq!(ring.peek(0), Some([1, -1]));
        ring.pop();
        ring.pop();
        assert!(ring.push(&[[4, -4], [i16::MIN, i16::MAX], [6, 6]]));
        assert_eq!(ring.len(), 4);
        let mut seen = Vec::new();
        while let Some(frame) = ring.peek(0) {
            seen.push(frame);
            ring.pop();
        }
        assert_eq!(seen, [[3, -3], [4, -4], [i16::MIN, i16::MAX], [6, 6]]);
        ring.pop();
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn the_converter_stays_in_range_and_keeps_the_rate() {
        for rate in [11_025, 22_050, 44_100, 48_000] {
            let mut converter = Converter::new(rate);
            let ring = Ring::new(22_050);
            let frames: Vec<[i16; 2]> = (0..22_050)
                .map(|i| {
                    if i % 2 == 0 {
                        [30_000; 2]
                    } else {
                        [-30_000; 2]
                    }
                })
                .collect();
            assert!(ring.push(&frames));
            let mut out = 0usize;
            while let Some([l, r]) = converter.next(&ring) {
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
