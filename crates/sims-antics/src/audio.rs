//! Deterministic clean-room PCM WAV synthesizer.
//!
//! Uses `libm` transcendental math and a deterministic xorshift32 noise
//! generator so output bytes are bit-identical across all platforms.

use crate::error::{Error, Result};
use std::f64::consts::TAU;

/// Default sample rate for synthesized open-pack audio effects (22.05 kHz mono 16-bit).
pub const SAMPLE_RATE: u32 = 22_050;
/// Minimum accepted custom sample rate (Hz).
pub const MIN_SAMPLE_RATE: u32 = 8_000;
/// Maximum accepted custom sample rate (Hz).
pub const MAX_SAMPLE_RATE: u32 = 192_000;
/// Maximum samples permitted in one synthesized WAV (60 seconds at 48 kHz = 2.88M samples).
pub const MAX_WAV_SAMPLES: usize = 60 * 48_000;

/// Built-in procedural sound effect presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundPreset {
    UiClick,
    Chime,
    WoodCreak,
    WaterFlush,
    SoftThud,
}

/// Custom parametric tone/noise synthesizer specification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynthSpec {
    pub sample_rate: u32,
    pub duration_ms: u32,
    pub start_hz: f64,
    pub end_hz: f64,
    pub harmonic_mix: f64,
    pub noise_mix: f64,
    pub attack_ms: u32,
    pub decay_rate: f64,
    pub amplitude: f64,
    pub seed: u32,
}

/// Synthesizes a built-in sound preset into complete RIFF/WAVE bytes.
#[must_use]
pub fn synthesize_preset(preset: SoundPreset) -> Vec<u8> {
    let spec = match preset {
        SoundPreset::UiClick => SynthSpec {
            sample_rate: SAMPLE_RATE,
            duration_ms: 45,
            start_hz: 1100.0,
            end_hz: 620.0,
            harmonic_mix: 0.25,
            noise_mix: 0.15,
            attack_ms: 2,
            decay_rate: 48.0,
            amplitude: 0.65,
            seed: 0x1A2B_3C4D,
        },
        SoundPreset::Chime => SynthSpec {
            sample_rate: SAMPLE_RATE,
            duration_ms: 360,
            start_hz: 660.0,
            end_hz: 880.0,
            harmonic_mix: 0.45,
            noise_mix: 0.0,
            attack_ms: 8,
            decay_rate: 7.5,
            amplitude: 0.60,
            seed: 0x2B3C_4D5E,
        },
        SoundPreset::WoodCreak => SynthSpec {
            sample_rate: SAMPLE_RATE,
            duration_ms: 220,
            start_hz: 240.0,
            end_hz: 165.0,
            harmonic_mix: 0.55,
            noise_mix: 0.22,
            attack_ms: 15,
            decay_rate: 12.0,
            amplitude: 0.55,
            seed: 0x3C4D_5E6F,
        },
        SoundPreset::WaterFlush => SynthSpec {
            sample_rate: SAMPLE_RATE,
            duration_ms: 420,
            start_hz: 320.0,
            end_hz: 120.0,
            harmonic_mix: 0.15,
            noise_mix: 0.78,
            attack_ms: 30,
            decay_rate: 5.0,
            amplitude: 0.58,
            seed: 0x4D5E_6F70,
        },
        SoundPreset::SoftThud => SynthSpec {
            sample_rate: SAMPLE_RATE,
            duration_ms: 140,
            start_hz: 150.0,
            end_hz: 62.0,
            harmonic_mix: 0.20,
            noise_mix: 0.30,
            attack_ms: 4,
            decay_rate: 24.0,
            amplitude: 0.70,
            seed: 0x5E6F_7081,
        },
    };
    synthesize_wav(&spec).expect("built-in audio preset satisfies all bounds")
}

/// Synthesizes PCM 16-bit mono RIFF/WAVE bytes from a [`SynthSpec`], validating
/// sample rate, duration, finiteness, and maximum sample budget before allocating.
pub fn synthesize_wav(spec: &SynthSpec) -> Result<Vec<u8>> {
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&spec.sample_rate) {
        return Err(Error::new(format!(
            "WAV sample_rate {} outside supported {MIN_SAMPLE_RATE}..={MAX_SAMPLE_RATE} Hz range",
            spec.sample_rate
        )));
    }
    if spec.duration_ms == 0 {
        return Err(Error::new("WAV duration_ms must be > 0"));
    }
    if !spec.start_hz.is_finite()
        || !spec.end_hz.is_finite()
        || !spec.harmonic_mix.is_finite()
        || !spec.noise_mix.is_finite()
        || !spec.decay_rate.is_finite()
        || !spec.amplitude.is_finite()
    {
        return Err(Error::new("SynthSpec contains non-finite float parameters"));
    }
    let sample_count_u64 = (u64::from(spec.sample_rate) * u64::from(spec.duration_ms)) / 1000;
    let sample_count = usize::try_from(sample_count_u64)
        .ok()
        .filter(|&n| n > 0 && n <= MAX_WAV_SAMPLES)
        .ok_or_else(|| {
            Error::new(format!(
                "synthesized WAV sample count {sample_count_u64} outside 1..={MAX_WAV_SAMPLES}"
            ))
        })?;
    let attack_samples = ((u64::from(spec.sample_rate) * u64::from(spec.attack_ms)) / 1000)
        .max(1)
        .min(sample_count as u64) as usize;

    let mut samples = Vec::with_capacity(sample_count);
    let mut phase = 0.0f64;
    let mut rng = spec.seed.max(1);
    let mut lp_noise = 0.0f64;
    let total_f = sample_count as f64;
    let dt = 1.0 / f64::from(spec.sample_rate);

    for i in 0..sample_count {
        let frac = i as f64 / total_f;
        let t = i as f64 * dt;
        let freq = spec.start_hz + (spec.end_hz - spec.start_hz) * frac;
        phase += TAU * freq * dt;

        let tone = libm::sin(phase) + spec.harmonic_mix * libm::sin(phase * 2.0);
        // Deterministic xorshift32 noise with gentle single-pole lowpass smoothing.
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let white = (rng as f64 / f64::from(u32::MAX)) * 2.0 - 1.0;
        lp_noise = lp_noise * 0.65 + white * 0.35;

        let env_attack = if i < attack_samples {
            i as f64 / attack_samples as f64
        } else {
            1.0
        };
        let env_decay = libm::exp(-spec.decay_rate * t);
        let tail_fade = if frac > 0.9 { (1.0 - frac) * 10.0 } else { 1.0 };
        let mixed = (tone * (1.0 - spec.noise_mix) + lp_noise * spec.noise_mix)
            * env_attack
            * env_decay
            * tail_fade;
        let clamped = (mixed * spec.amplitude).clamp(-1.0, 1.0);
        samples.push((clamped * f64::from(i16::MAX)) as i16);
    }

    encode_pcm16_wav(spec.sample_rate, &samples)
}

/// Encodes mono 16-bit little-endian PCM samples into a valid RIFF/WAVE buffer,
/// checking sample rate, byte rate, and RIFF chunk size limits before allocating.
pub fn encode_pcm16_wav(sample_rate: u32, samples: &[i16]) -> Result<Vec<u8>> {
    if sample_rate == 0 {
        return Err(Error::new("WAV sample_rate must be non-zero"));
    }
    let byte_rate = sample_rate
        .checked_mul(2)
        .ok_or_else(|| Error::new("WAV byte_rate overflows u32"))?;
    let data_bytes_usize = samples
        .len()
        .checked_mul(2)
        .ok_or_else(|| Error::new("WAV PCM data byte length overflows usize"))?;
    let data_bytes = u32::try_from(data_bytes_usize)
        .map_err(|_| Error::new("WAV PCM data exceeds 32-bit RIFF limit"))?;
    let riff_size = 36u32
        .checked_add(data_bytes)
        .ok_or_else(|| Error::new("WAV RIFF chunk size overflows u32"))?;
    let total_len = 44usize
        .checked_add(data_bytes_usize)
        .ok_or_else(|| Error::new("WAV total output length overflows usize"))?;

    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    // fmt chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt length
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format tag
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    Ok(out)
}
