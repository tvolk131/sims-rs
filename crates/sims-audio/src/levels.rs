//! Volume, pan and pitch, from a track's registers to gains and a
//! playback rate.
//!
//! The formulas follow the original's code, read from a memory image of
//! the running game (not traced at run time):
//!
//! - **Volume.** `v = vol * extvol / 1024`, where `extvol` is first scaled
//!   by the track's group volume (`* group / 1024`) when that is not 1024.
//!   When `main_duckpri` (global register 123) is not 0 and the track's
//!   `duckpri` is below it, `v` is halved. The sound layer clamps `v` to
//!   0..1024, scales it by the master volume (`* master / 1024`), and
//!   hands DirectSound an attenuation in hundredths of a decibel: -10000
//!   for 0, 0 for 1024, and `v * 4000 / 1024 - 4000` between. All
//!   divisions truncate toward zero, and the products for `v` wrap in 32
//!   bits, as the original's do.
//! - **Pan.** Clamped to 0..1024, then `pan * 20000 / 1024 - 10000`
//!   hundredths of a decibel: below 0 the right channel is attenuated by
//!   that much, above 0 the left. 512 is the centre.
//! - **Pitch.** A playback rate of `22050 * 2^((pitch - 3600) / 1200)`
//!   frames per second, whatever the sample's own rate, clamped to 100 to
//!   100,000. Pitch 3600 plays at exactly 22,050 Hz. The original adds a
//!   second track's pitch and subtracts 3600; which track that is was not
//!   found, and it is taken to hold 3600. Every patch in the install is
//!   22,050 Hz, and the television routines set pitch 2400 for files of
//!   11,025 Hz, which this formula plays at their own rate.
//!
//! DirectSound's attenuation is turned into a linear gain as
//! `10^(hundredths / 2000)`, with -10000 or less as silence. Where the
//! original then rounds is DirectSound's business and is not modeled.
//! Gains are 16.16 fixed point; a gain of 1 is exactly 65,536, so a track
//! at full volume, centred, passes its samples through unchanged.
//!
//! The exponentials use only addition, multiplication and division, so
//! every platform computes the same gains bit for bit.

/// A linear gain of 1 in 16.16 fixed point.
pub const UNITY: i32 = 1 << 16;
/// The output rate, and the rate at pitch 3600.
pub const BASE_RATE: u32 = 22_050;
/// The pitch at which a sample plays at [`BASE_RATE`].
pub const BASE_PITCH: i32 = 3600;

/// `e^x` for the small arguments this module needs, with no library call.
fn exp(x: f64) -> f64 {
    const LN2: f64 = std::f64::consts::LN_2;
    // e^x = 2^n * e^r, with |r| <= ln 2 / 2.
    let n = (x / LN2).round();
    let r = x - n * LN2;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..=24 {
        term = term * r / f64::from(k);
        sum += term;
    }
    let mut scale = 1.0;
    let mut n = n as i32;
    while n > 0 {
        scale *= 2.0;
        n -= 1;
    }
    while n < 0 {
        scale *= 0.5;
        n += 1;
    }
    sum * scale
}

/// The DirectSound attenuation for a volume, as the original computes it.
pub fn attenuation(volume: i64, master: i64) -> i32 {
    let v = volume.clamp(0, 1024) * master.clamp(0, 1024) / 1024;
    match v {
        0 => -10_000,
        1024 => 0,
        v => (v * 4000 / 1024 - 4000) as i32,
    }
}

/// The DirectSound pan for a `pan` register.
pub fn pan(pan: i32) -> i32 {
    (i64::from(pan).clamp(0, 1024) * 20_000 / 1024 - 10_000) as i32
}

/// A gain in 16.16 fixed point for an attenuation in hundredths of a
/// decibel (0 or less).
pub fn gain(hundredths: i32) -> i32 {
    match hundredths {
        h if h >= 0 => UNITY,
        h if h <= -10_000 => 0,
        h => (exp(f64::from(h) * std::f64::consts::LN_10 / 2000.0) * f64::from(UNITY)) as i32,
    }
}

/// Left and right gains for an attenuation and a DirectSound pan.
pub fn gains(attenuation: i32, pan: i32) -> [i32; 2] {
    let level = i64::from(gain(attenuation));
    let left = i64::from(gain(-pan.max(0)));
    let right = i64::from(gain(pan.min(0)));
    [
        ((level * left) >> 16) as i32,
        ((level * right) >> 16) as i32,
    ]
}

/// The playback rate, in frames per second, for a `pitch` register.
pub fn rate(pitch: i32) -> u32 {
    let cents = i64::from(pitch) - i64::from(BASE_PITCH);
    if cents == 0 {
        return BASE_RATE;
    }
    // Whole octaves are exact; the rest is at most one octave.
    let octaves = cents.div_euclid(1200).clamp(-64, 64);
    let rest = cents.rem_euclid(1200);
    let mut rate = f64::from(BASE_RATE) * exp(rest as f64 * std::f64::consts::LN_2 / 1200.0);
    for _ in 0..octaves.abs() {
        rate = if octaves > 0 { rate * 2.0 } else { rate * 0.5 };
    }
    (rate.clamp(100.0, 100_000.0)) as u32
}

/// The registers that set a track's level, and the settings around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub vol: i32,
    pub extvol: i32,
    pub pan: i32,
    pub pitch: i32,
    pub duck_priority: i32,
    /// Register 26: 3D sound, which sets a position instead of a pan.
    pub three_d: bool,
    /// The track group's volume (1024 when not set).
    pub group_volume: i32,
    /// Global register 123.
    pub main_duck_priority: i32,
    pub master: i32,
}

/// What a note plays at: left and right gains in 16.16 fixed point, and
/// a playback rate in frames per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels {
    pub gains: [i32; 2],
    pub rate: u32,
}

impl Levels {
    /// Full volume, centred, at the base rate.
    pub const UNITY: Self = Self {
        gains: [UNITY, UNITY],
        rate: BASE_RATE,
    };
}

/// The volume `v` the original hands its sound layer, before the master
/// volume. The original multiplies in 32 bits, so products past the range
/// wrap, as they do here.
pub fn volume(inputs: &Inputs) -> i32 {
    let mut extvol = inputs.extvol;
    if inputs.group_volume != 1024 {
        extvol = inputs.group_volume.wrapping_mul(extvol) / 1024;
    }
    let mut v = inputs.vol.wrapping_mul(extvol) / 1024;
    if inputs.main_duck_priority != 0 && inputs.duck_priority < inputs.main_duck_priority {
        v /= 2;
    }
    v
}

/// A track's levels. A 3D track is centred: 3D position is not modeled.
pub fn levels(inputs: &Inputs) -> Levels {
    let attenuation = attenuation(i64::from(volume(inputs)), i64::from(inputs.master));
    let pan = if inputs.three_d { 0 } else { pan(inputs.pan) };
    Levels {
        gains: gains(attenuation, pan),
        rate: rate(inputs.pitch),
    }
}
