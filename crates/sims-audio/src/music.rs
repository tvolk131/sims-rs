//! Neighborhood, Buy, Build, Load, and expansion mode music playback.
//!
//! *(code)* Complete Collection 1.91 switches background music through event
//! kind 36 (`kSetMusicMode`, `0x573fe2` → `0x575170`), which steers
//! `MainMusicLoop` (track 515), `SetMainMusicSongAndHitList` (516),
//! `SetMainAndHitListKillFirst` (517), and `FadeAndKillTrackSlow` (519) in
//! `SoundData/SimsSound.hit`. Each mode maps to a station directory under
//! `Music/Modes/` (registered at `0x5718b2..0x571a12`), using `NhoodUS` when
//! language is US English (`[eax+0x64] == 1`) and `Nhood` otherwise.
//!
//! This module implements a dedicated streaming [`ModeMusicPlayer`] over
//! [`SoundFiles`] that:
//!
//! - discovers every mode's MP3/WAV tracks from `Music/Modes/*` (and Unleashed's
//!   `Music/Stations/UnlSpeak`),
//! - streams MP3 tracks incrementally packet-by-packet via `symphonia` (avoiding
//!   full-song synchronous decodes into ~45 MB PCM buffers) and resamples to
//!   `output_rate` (default [`crate::player::RATE`], 22,050 Hz stereo),
//! - reproduces the 1.91 `kSetMusicMode` transition matrix, non-repeating
//!   shuffled playlists, smooth crossfades, and `FadeAndKillTrackSlow`
//!   (`fade_speed = 3` per 20 ms HIT tick ≈ 6.84 s), and
//! - scales output by the exact 1.91 Music volume slider table
//!   ([`VOLUME_STEPS`], `0x67156c`, `kind 38` / `kGroupMusic` group 2).

use crate::files::{SoundFiles, Source, key};
use crate::player::{RATE, VOLUME_STEPS};
use crate::{Error, Pcm, Result, wav};
use std::collections::BTreeMap;
use symphonia_bundle_mp3::{MpaDecoder, MpaReader};
use symphonia_core::audio::SampleBuffer;
use symphonia_core::codecs::{Decoder, DecoderOptions};
use symphonia_core::errors::Error as SymphoniaError;
use symphonia_core::formats::{FormatOptions, FormatReader};
use symphonia_core::io::{MediaSourceStream, MediaSourceStreamOptions};

/// HIT timer tick rate (50 Hz = 20 ms per tick).
pub const HIT_TICK_HZ: u32 = 50;
/// `FadeAndKillTrackSlow` fade step per 20 ms tick (`SimsSound.hit` `0x191`:
/// `fade_speed = 3`, reducing volume `1024 -> 0` in 342 ticks ≈ 6.84 s).
pub const SLOW_FADE_STEP_PER_TICK: i32 = 3;
/// Default crossfade duration in milliseconds when switching directly between
/// two active music modes (e.g., Buy ↔ Build).
pub const DEFAULT_CROSSFADE_MS: u32 = 250;
/// Maximum music gain (`0..=1024`, matching HIT `vol` and `kGroupMusic`).
pub const MAX_VOLUME: i32 = 1024;
/// Default expansion bitmask (`[0x6e48bc]+0x66`, all Complete Collection packs).
pub const ALL_EXPANSIONS: u8 = 0xff;

/// Every `kSetMusicMode` mode number in Complete Collection 1.91 (`0x575170`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum MusicMode {
    /// Mode 0: Live lot gameplay (stops Buy/Build immediately, slowly fades out
    /// Neighborhood / expansion music).
    Live = 0,
    /// Mode 1: Buy Mode (`Music\Modes\Buy`, hit list 221).
    Buy = 1,
    /// Mode 2: Build Mode (`Music\Modes\Build`, hit list 222).
    Build = 2,
    /// Mode 3: Neighborhood screen (`Music\Modes\NhoodUS` or `Nhood`, hit list 223).
    Neighborhood = 3,
    /// Mode 4: Silent / uninitialized default (`0x571420`).
    None = 4,
    /// Mode 5: Loading screen (`Music\Modes\Load\loadloop.wav`).
    Load = 5,
    /// Mode 6: Credits screen (newest installed expansion's station).
    Credits = 6,
    /// Mode 7: Options screen (leaves active music playing).
    Options = 7,
    /// Mode 8: Family Bin / Create-a-Family (fades ambient loops, keeps music).
    FamilyBin = 8,
    /// Mode 9: Explicit slow fade-out (`bkground_fade`).
    FadeOut = 9,
    /// Mode 10: Hot Date Downtown (`Music\Modes\dt_nhood`, hit list 6100).
    Downtown = 10,
    /// Mode 11: Vacation Island (`Music\Modes\Vacation`, hit list 14093).
    Vacation = 11,
    /// Mode 12: Unleashed Old Town (`Music\Stations\UnlSpeak`, hit list 20902).
    Unleashed = 12,
    /// Mode 13: Superstar Studio Town (`Music\Modes\SuperStar`, hit list 100102).
    Superstar = 13,
    /// Mode 14: Superstar transition (`Music\Stations\Superstar`, hit list 100101).
    SuperstarTransition = 14,
    /// Mode 15: Makin' Magic Magic Town (`Music\Modes\MakinMagic`, hit list 103100).
    MagicTown = 15,
    /// Mode 16: Makin' Magic credits (`Music\Modes\MM_Credits`, hit list 103101).
    MagicTownCredits = 16,
    /// Mode 17: Makin' Magic Build Mode (`Music\Modes\MM_Build`, hit list 103102).
    MagicTownBuild = 17,
    /// Mode 18: Makin' Magic Buy Mode (`Music\Modes\MM_Buy`, hit list 103104).
    MagicTownBuy = 18,
}

impl MusicMode {
    /// All modes that own a playable track directory.
    pub const PLAYABLE: [Self; 13] = [
        Self::Buy,
        Self::Build,
        Self::Neighborhood,
        Self::Load,
        Self::Downtown,
        Self::Vacation,
        Self::Unleashed,
        Self::Superstar,
        Self::SuperstarTransition,
        Self::MagicTown,
        Self::MagicTownCredits,
        Self::MagicTownBuild,
        Self::MagicTownBuy,
    ];

    /// Converts a 1.91 `kSetMusicMode` integer (`0..=18`) into a [`MusicMode`].
    pub const fn from_id(id: i64) -> Option<Self> {
        match id {
            0 => Some(Self::Live),
            1 => Some(Self::Buy),
            2 => Some(Self::Build),
            3 => Some(Self::Neighborhood),
            4 => Some(Self::None),
            5 => Some(Self::Load),
            6 => Some(Self::Credits),
            7 => Some(Self::Options),
            8 => Some(Self::FamilyBin),
            9 => Some(Self::FadeOut),
            10 => Some(Self::Downtown),
            11 => Some(Self::Vacation),
            12 => Some(Self::Unleashed),
            13 => Some(Self::Superstar),
            14 => Some(Self::SuperstarTransition),
            15 => Some(Self::MagicTown),
            16 => Some(Self::MagicTownCredits),
            17 => Some(Self::MagicTownBuild),
            18 => Some(Self::MagicTownBuy),
            _ => None,
        }
    }

    /// The 1.91 `kSetMusicMode` numeric ID (`0..=18`).
    pub const fn id(self) -> i64 {
        self as i64
    }

    /// Canonical HOT event name triggering this mode (`SimsSound.hot` and
    /// expansion `.hot` files).
    pub const fn event_name(self) -> Option<&'static str> {
        match self {
            Self::Live => Some("bkground_live"),
            Self::Buy => Some("bkground_buy1"),
            Self::Build => Some("bkground_build"),
            Self::Neighborhood => Some("bkground_nhood1"),
            Self::None => None,
            Self::Load => Some("bkground_load"),
            Self::Credits => Some("bkground_credits"),
            Self::Options => Some("bkground_options"),
            Self::FamilyBin => Some("bkground_family"),
            Self::FadeOut => Some("bkground_fade"),
            Self::Downtown => Some("station_dtnhood"),
            Self::Vacation => Some("station_vacation"),
            Self::Unleashed => Some("station_unleashed"),
            Self::Superstar => Some("station_superstar"),
            Self::SuperstarTransition => Some("music_superstar_transition"),
            Self::MagicTown => Some("music_magictown"),
            Self::MagicTownCredits => Some("music_magictown_credits"),
            Self::MagicTownBuild => Some("music_magictown_build"),
            Self::MagicTownBuy => Some("music_magictown_buy"),
        }
    }

    /// The HIT station list ID for this mode (`0x575211..0x5755fb`).
    pub const fn hit_list(self, expansions: u8) -> Option<u32> {
        match self {
            Self::Buy => Some(221),
            Self::Build => Some(222),
            Self::Neighborhood => Some(223),
            Self::Credits => Some(credits_hit_list(expansions)),
            Self::Downtown => Some(6_100),
            Self::Vacation => Some(14_093),
            Self::Unleashed => Some(20_902),
            Self::Superstar => Some(100_102),
            Self::SuperstarTransition => Some(100_101),
            Self::MagicTown => Some(103_100),
            Self::MagicTownCredits => Some(103_101),
            Self::MagicTownBuild => Some(103_102),
            Self::MagicTownBuy => Some(103_104),
            Self::Live
            | Self::None
            | Self::Load
            | Self::Options
            | Self::FamilyBin
            | Self::FadeOut => None,
        }
    }

    /// Install-relative directory supplying this mode's audio files
    /// (`0x5718b2..0x571a12`). `us_english` selects `NhoodUS` vs `Nhood`.
    pub const fn directory(self, us_english: bool, expansions: u8) -> Option<&'static str> {
        match self {
            Self::Buy => Some("Music\\Modes\\Buy"),
            Self::Build => Some("Music\\Modes\\Build"),
            Self::Neighborhood => Some(if us_english {
                "Music\\Modes\\NhoodUS"
            } else {
                "Music\\Modes\\Nhood"
            }),
            Self::Load => Some("Music\\Modes\\Load"),
            Self::Credits => Some(credits_directory(us_english, expansions)),
            Self::Downtown => Some("Music\\Modes\\dt_nhood"),
            Self::Vacation => Some("Music\\Modes\\Vacation"),
            Self::Unleashed => Some("Music\\Stations\\UnlSpeak"),
            Self::Superstar => Some("Music\\Modes\\SuperStar"),
            Self::SuperstarTransition => Some("Music\\Stations\\Superstar"),
            Self::MagicTown => Some("Music\\Modes\\MakinMagic"),
            Self::MagicTownCredits => Some("Music\\Modes\\MM_Credits"),
            Self::MagicTownBuild => Some("Music\\Modes\\MM_Build"),
            Self::MagicTownBuy => Some("Music\\Modes\\MM_Buy"),
            Self::Live | Self::None | Self::Options | Self::FamilyBin | Self::FadeOut => None,
        }
    }
}

const fn credits_hit_list(expansions: u8) -> u32 {
    if expansions & 0x80 != 0 {
        103_101
    } else if expansions & 0x40 != 0 {
        100_102
    } else if expansions & 0x20 != 0 {
        20_902
    } else if expansions & 0x08 != 0 {
        14_093
    } else if expansions & 0x02 != 0 {
        1_589
    } else if expansions & 0x04 != 0 {
        6_100
    } else {
        223
    }
}

const fn credits_directory(us_english: bool, expansions: u8) -> &'static str {
    if expansions & 0x80 != 0 {
        "Music\\Modes\\MM_Credits"
    } else if expansions & 0x40 != 0 {
        "Music\\Modes\\SuperStar"
    } else if expansions & 0x20 != 0 {
        "Music\\Stations\\UnlSpeak"
    } else if expansions & 0x08 != 0 {
        "Music\\Modes\\Vacation"
    } else if expansions & 0x02 != 0 {
        "Music\\Stations\\Disco"
    } else if expansions & 0x04 != 0 {
        "Music\\Modes\\dt_nhood"
    } else if us_english {
        "Music\\Modes\\NhoodUS"
    } else {
        "Music\\Modes\\Nhood"
    }
}

/// Discovered track lists per [`MusicMode`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModeCatalog {
    pub us_english: bool,
    pub expansions: u8,
    tracks: BTreeMap<MusicMode, Vec<String>>,
}

impl ModeCatalog {
    /// Builds a catalog from `files` (scanning loose files in each mode's
    /// directory, matching 1.91's disk scan at `0x574c37`).
    pub fn from_sound_files(files: &SoundFiles, us_english: bool, expansions: u8) -> Self {
        let mut by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for entry in files.iter() {
            if !matches!(entry.source, Source::Loose(_)) {
                continue;
            }
            let normalized = key(&entry.path);
            let Some((dir, _)) = normalized.rsplit_once('\\') else {
                continue;
            };
            by_dir
                .entry(dir.to_owned())
                .or_default()
                .push(entry.path.clone());
        }
        for list in by_dir.values_mut() {
            list.sort_by_key(|p| key(p));
        }

        let mut tracks = BTreeMap::new();
        for mode in MusicMode::PLAYABLE {
            if let Some(dir) = mode.directory(us_english, expansions)
                && let Some(paths) = by_dir.get(&key(dir))
                && !paths.is_empty()
            {
                tracks.insert(mode, paths.clone());
            }
        }
        // Credits resolves dynamically to the newest expansion's directory.
        if let Some(dir) = MusicMode::Credits.directory(us_english, expansions)
            && let Some(paths) = by_dir.get(&key(dir))
            && !paths.is_empty()
        {
            tracks.insert(MusicMode::Credits, paths.clone());
        }

        Self {
            us_english,
            expansions,
            tracks,
        }
    }

    /// Creates an empty catalog (for synthetic tests using [`Self::insert`]).
    pub fn empty() -> Self {
        Self {
            us_english: true,
            expansions: ALL_EXPANSIONS,
            tracks: BTreeMap::new(),
        }
    }

    /// Registers explicit track paths for `mode`.
    pub fn insert(&mut self, mode: MusicMode, paths: Vec<String>) {
        if !paths.is_empty() {
            self.tracks.insert(mode, paths);
        }
    }

    /// Returns the discovered tracks for `mode` in deterministic key order.
    pub fn tracks(&self, mode: MusicMode) -> &[String] {
        self.tracks.get(&mode).map_or(&[], Vec::as_slice)
    }
}

/// Provider of raw audio file bytes (MP3 or WAV) for [`ModeMusicPlayer`].
pub trait TrackSource {
    /// Reads the raw bytes of the file at `path`.
    fn read_bytes(&mut self, path: &str) -> Option<Vec<u8>>;
}

impl TrackSource for SoundFiles {
    fn read_bytes(&mut self, path: &str) -> Option<Vec<u8>> {
        self.get(path)?.read().ok()
    }
}

impl TrackSource for BTreeMap<String, Vec<u8>> {
    fn read_bytes(&mut self, path: &str) -> Option<Vec<u8>> {
        self.get(path).cloned()
    }
}

/// An active streaming audio track (incremental MP3 packet decoder or looping
/// PCM buffer for WAV files such as `loadloop.wav`), resampled on the fly to
/// `output_rate` stereo.
struct StreamDeck {
    path: String,
    inner: StreamInner,
    /// Decoded stereo interleaved source frames (`[L, R, L, R, ...]`).
    buffer: Vec<i16>,
    /// Source sample rate in Hz.
    source_rate: u32,
    /// Fixed-point 32.32 frame cursor into `buffer`.
    phase: u64,
    /// Fixed-point 32.32 source frames consumed per output frame.
    step: u64,
    /// Whether the underlying source has reached EOF.
    eof: bool,
    /// Current per-deck fade volume (`0..=1024`, scaled in 16.16 fixed point
    /// for smooth per-frame ramps).
    gain_fp: i64,
    /// Target per-deck fade volume in 16.16 fixed point.
    target_fp: i64,
    /// Signed per-frame ramp delta in 16.16 fixed point.
    ramp_per_frame: i64,
    /// Drop this deck once its fade reaches 0.
    stop_at_zero: bool,
}

enum StreamInner {
    Mp3 {
        reader: Box<MpaReader>,
        decoder: Box<MpaDecoder>,
    },
    Pcm {
        pcm: Pcm,
        cursor: usize,
    },
}

impl StreamDeck {
    fn open(path: String, bytes: Vec<u8>, output_rate: u32) -> Result<Self> {
        let output_rate = output_rate.max(1);
        if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            let pcm = wav::decode(&bytes)?;
            if pcm.rate == 0 || pcm.channels == 0 || pcm.frames() == 0 {
                return Err(Error::format("WAV", 0, "empty music track"));
            }
            let source_rate = pcm.rate;
            let step = ((u64::from(source_rate) << 32) / u64::from(output_rate)).max(1);
            let mut deck = Self {
                path,
                inner: StreamInner::Pcm { pcm, cursor: 0 },
                buffer: Vec::new(),
                source_rate,
                phase: 0,
                step,
                eof: false,
                gain_fp: i64::from(MAX_VOLUME) << 16,
                target_fp: i64::from(MAX_VOLUME) << 16,
                ramp_per_frame: 0,
                stop_at_zero: false,
            };
            deck.refill();
            return Ok(deck);
        }

        let source = MediaSourceStream::new(
            Box::new(std::io::Cursor::new(bytes)),
            MediaSourceStreamOptions::default(),
        );
        let reader = MpaReader::try_new(source, &FormatOptions::default())
            .map_err(|e| Error::format("MP3", 0, e.to_string()))?;
        let params = reader
            .default_track()
            .ok_or_else(|| Error::format("MP3", 0, "no audio track"))?
            .codec_params
            .clone();
        let decoder = MpaDecoder::try_new(&params, &DecoderOptions::default())
            .map_err(|e| Error::format("MP3", 0, e.to_string()))?;
        let source_rate = params.sample_rate.unwrap_or(44_100).max(1);
        let step = ((u64::from(source_rate) << 32) / u64::from(output_rate)).max(1);

        let mut deck = Self {
            path,
            inner: StreamInner::Mp3 {
                reader: Box::new(reader),
                decoder: Box::new(decoder),
            },
            buffer: Vec::with_capacity(4608),
            source_rate,
            phase: 0,
            step,
            eof: false,
            gain_fp: i64::from(MAX_VOLUME) << 16,
            target_fp: i64::from(MAX_VOLUME) << 16,
            ramp_per_frame: 0,
            stop_at_zero: false,
        };
        deck.refill();
        Ok(deck)
    }

    fn start_fade(&mut self, target_vol: i32, duration_frames: u32, stop_at_zero: bool) {
        let target_fp = i64::from(target_vol.clamp(0, MAX_VOLUME)) << 16;
        self.target_fp = target_fp;
        self.stop_at_zero = stop_at_zero;
        if duration_frames == 0 || self.gain_fp == target_fp {
            self.gain_fp = target_fp;
            self.ramp_per_frame = 0;
        } else {
            let diff = target_fp - self.gain_fp;
            let delta = diff / i64::from(duration_frames);
            self.ramp_per_frame = if delta == 0 { diff.signum() } else { delta };
        }
    }

    fn is_finished(&self) -> bool {
        if self.stop_at_zero && self.gain_fp <= 0 {
            return true;
        }
        let frames_in_buf = (self.buffer.len() / 2) as u64;
        self.eof && (self.phase >> 32) >= frames_in_buf
    }

    fn refill(&mut self) {
        // Discard fully consumed frames from the front of `buffer`.
        let consumed = (self.phase >> 32) as usize;
        if consumed > 0 {
            let available = self.buffer.len() / 2;
            let drop_frames = consumed.min(available);
            self.buffer.drain(..drop_frames * 2);
            self.phase -= (drop_frames as u64) << 32;
        }

        // Keep at least 2048 stereo source frames buffered for interpolation.
        while !self.eof && self.buffer.len() / 2 < 2048 {
            match &mut self.inner {
                StreamInner::Pcm { pcm, cursor } => {
                    let total = pcm.frames();
                    if *cursor >= total {
                        self.eof = true;
                        break;
                    }
                    let take = (total - *cursor).min(2048);
                    let ch = usize::from(pcm.channels);
                    for f in *cursor..(*cursor + take) {
                        let base = f * ch;
                        let left = pcm.samples[base];
                        let right = if ch >= 2 { pcm.samples[base + 1] } else { left };
                        self.buffer.push(left);
                        self.buffer.push(right);
                    }
                    *cursor += take;
                }
                StreamInner::Mp3 { reader, decoder } => match reader.next_packet() {
                    Ok(packet) => match decoder.decode(&packet) {
                        Ok(decoded) => {
                            let spec = *decoded.spec();
                            let ch = spec.channels.count();
                            if ch == 0 {
                                continue;
                            }
                            if spec.rate > 0 && spec.rate != self.source_rate {
                                // Update source rate if header differed from first packet.
                                let ratio_num = u128::from(self.step) * u128::from(spec.rate);
                                self.step =
                                    (ratio_num / u128::from(self.source_rate.max(1))).max(1) as u64;
                                self.source_rate = spec.rate;
                            }
                            let mut sample_buf =
                                SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
                            sample_buf.copy_interleaved_ref(decoded);
                            let samples = sample_buf.samples();
                            for frame in samples.chunks_exact(ch) {
                                let left = frame[0];
                                let right = if ch >= 2 { frame[1] } else { left };
                                self.buffer.push(left);
                                self.buffer.push(right);
                            }
                        }
                        Err(SymphoniaError::DecodeError(_)) => continue,
                        Err(_) => {
                            self.eof = true;
                            break;
                        }
                    },
                    Err(_) => {
                        self.eof = true;
                        break;
                    }
                },
            }
        }
    }

    /// Reads one resampled stereo sample pair and advances the deck's fade ramp.
    fn next_frame(&mut self) -> Option<[i32; 2]> {
        if self.stop_at_zero && self.gain_fp <= 0 {
            return None;
        }
        let mut idx = (self.phase >> 32) as usize;
        if idx + 1 >= self.buffer.len() / 2 && !self.eof {
            self.refill();
            idx = (self.phase >> 32) as usize;
        }
        let frames = self.buffer.len() / 2;
        if idx >= frames {
            return None;
        }
        let next = (idx + 1).min(frames - 1);
        let frac = (self.phase & 0xffff_ffff) as i64;
        let l0 = i64::from(self.buffer[idx * 2]);
        let r0 = i64::from(self.buffer[idx * 2 + 1]);
        let l1 = i64::from(self.buffer[next * 2]);
        let r1 = i64::from(self.buffer[next * 2 + 1]);
        let left = l0 + (((l1 - l0) * frac) >> 32);
        let right = r0 + (((r1 - r0) * frac) >> 32);

        self.phase = self.phase.wrapping_add(self.step);

        // Apply per-deck fade volume (`0..=1024`).
        let vol = (self.gain_fp >> 16).clamp(0, i64::from(MAX_VOLUME));
        if self.ramp_per_frame != 0 {
            let next_fp = self.gain_fp + self.ramp_per_frame;
            if (self.ramp_per_frame > 0 && next_fp >= self.target_fp)
                || (self.ramp_per_frame < 0 && next_fp <= self.target_fp)
            {
                self.gain_fp = self.target_fp;
                self.ramp_per_frame = 0;
            } else {
                self.gain_fp = next_fp;
            }
        }

        Some([((left * vol) >> 10) as i32, ((right * vol) >> 10) as i32])
    }
}

/// Streaming mode music controller implementing 1.91's Neighborhood, Buy,
/// Build, Load, and expansion music behavior.
pub struct ModeMusicPlayer<S> {
    catalog: ModeCatalog,
    source: S,
    output_rate: u32,
    mode: MusicMode,
    /// The station mode whose playlist is currently advancing (preserved across
    /// `Options` and `FamilyBin`).
    active_station: Option<MusicMode>,
    current: Option<StreamDeck>,
    /// Previous deck fading out during a crossfade.
    fading_out: Option<StreamDeck>,
    /// Per-mode shuffled playlist state + last track played (to prevent
    /// immediate repeats across reshuffles).
    playlists: BTreeMap<MusicMode, PlaylistState>,
    /// Current music volume slider step (`0..=10`, matching `kind 38` /
    /// `VOLUME_STEPS` at `0x67156c`).
    volume_step: u8,
    /// Effective music group volume (`0..=1024`).
    volume: i32,
    /// Master volume (`0..=1024`).
    master_volume: i32,
    /// Crossfade duration in milliseconds when switching between active modes.
    crossfade_ms: u32,
    paused: bool,
    rng: u64,
    /// Non-fatal file/decode diagnostics.
    pub faults: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct PlaylistState {
    queue: Vec<usize>,
    last_played: Option<usize>,
}

impl<S: TrackSource> ModeMusicPlayer<S> {
    /// Creates a new streaming mode music player at [`RATE`] (22,050 Hz) with
    /// deterministic PRNG `seed`.
    pub fn new(catalog: ModeCatalog, source: S, seed: u64) -> Self {
        Self::with_rate(catalog, source, RATE, seed)
    }

    /// Creates a new streaming mode music player targeting `output_rate` Hz.
    pub fn with_rate(catalog: ModeCatalog, source: S, output_rate: u32, seed: u64) -> Self {
        Self {
            catalog,
            source,
            output_rate: output_rate.max(1),
            mode: MusicMode::None,
            active_station: None,
            current: None,
            fading_out: None,
            playlists: BTreeMap::new(),
            volume_step: 10,
            volume: MAX_VOLUME,
            master_volume: MAX_VOLUME,
            crossfade_ms: DEFAULT_CROSSFADE_MS,
            paused: false,
            rng: splitmix64(seed),
            faults: Vec::new(),
        }
    }

    pub fn mode(&self) -> MusicMode {
        self.mode
    }

    pub fn active_station(&self) -> Option<MusicMode> {
        self.active_station
    }

    pub fn current_track(&self) -> Option<&str> {
        self.current.as_ref().map(|d| d.path.as_str())
    }

    pub fn is_playing(&self) -> bool {
        self.current.is_some() || self.fading_out.is_some()
    }

    pub fn is_fading_out(&self) -> bool {
        self.current.as_ref().is_some_and(|d| d.stop_at_zero)
    }

    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }

    pub fn catalog(&self) -> &ModeCatalog {
        &self.catalog
    }

    /// Sets crossfade duration in milliseconds when switching between active
    /// music stations (`0` for hard cuts).
    pub fn set_crossfade_ms(&mut self, ms: u32) {
        self.crossfade_ms = ms;
    }

    /// Sets the Music volume slider step (`0..=10`, clamped), using the exact
    /// 1.91 `VOLUME_STEPS` curve (`0x67156c`, `kind 38` / `kGroupMusic`).
    pub fn set_volume_step(&mut self, step: usize) {
        let clamped = step.min(VOLUME_STEPS.len() - 1);
        self.volume_step = clamped as u8;
        self.volume = VOLUME_STEPS[clamped];
    }

    pub fn volume_step(&self) -> u8 {
        self.volume_step
    }

    /// Sets the music group volume directly (`0..=1024`, clamped).
    pub fn set_volume(&mut self, volume: i32) {
        self.volume = volume.clamp(0, MAX_VOLUME);
    }

    pub fn volume(&self) -> i32 {
        self.volume
    }

    /// Sets the master volume (`0..=1024`, clamped).
    pub fn set_master_volume(&mut self, volume: i32) {
        self.master_volume = volume.clamp(0, MAX_VOLUME);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Transitions to `mode` following Complete Collection 1.91's
    /// `kSetMusicMode` (`0x575170`) rules.
    pub fn set_mode(&mut self, mode: MusicMode) {
        if mode == self.mode {
            return;
        }
        let previous = std::mem::replace(&mut self.mode, mode);

        match mode {
            // Options and FamilyBin keep the active music track playing
            // (`0x5755fd`: FamilyBin only fades ambient loops; Options is a no-op).
            MusicMode::Options | MusicMode::FamilyBin => {}

            // Live / None / FadeOut: Buy & Build stop immediately; everything
            // else runs `FadeAndKillTrackSlow` (`fade_speed = 3` per 20 ms tick).
            MusicMode::Live | MusicMode::None | MusicMode::FadeOut => {
                self.active_station = None;
                let immediate = matches!(
                    previous,
                    MusicMode::Buy
                        | MusicMode::Build
                        | MusicMode::MagicTownBuy
                        | MusicMode::MagicTownBuild
                );
                if immediate {
                    self.current = None;
                    self.fading_out = None;
                } else {
                    let slow_frames = self.slow_fade_frames();
                    if let Some(deck) = self.current.as_mut() {
                        deck.start_fade(0, slow_frames, true);
                    }
                }
            }

            // Returning from FamilyBin to Neighborhood keeps the neighborhood
            // music playing uninterrupted (`0x575263`).
            MusicMode::Neighborhood
                if previous == MusicMode::FamilyBin && self.current.is_some() =>
            {
                let crossfade = self.crossfade_frames();
                if let Some(deck) = self.current.as_mut()
                    && deck.stop_at_zero
                {
                    deck.start_fade(MAX_VOLUME, crossfade, false);
                }
                self.active_station = Some(MusicMode::Neighborhood);
            }

            // Makin' Magic Build <-> Buy keeps playing (`0x575467..0x5754bf`).
            MusicMode::MagicTownBuild | MusicMode::MagicTownBuy
                if matches!(
                    previous,
                    MusicMode::MagicTownBuild | MusicMode::MagicTownBuy
                ) && self.current.is_some() => {}

            // Switch to a playable station mode.
            station => {
                let crossfade = self.crossfade_frames();
                let mut old = self.current.take();
                self.active_station = Some(station);
                if self.start_next_track(station, crossfade > 0 && old.is_some()) {
                    if let Some(mut prev_deck) = old.take() {
                        if crossfade > 0 {
                            prev_deck.start_fade(0, crossfade, true);
                            self.fading_out = Some(prev_deck);
                        } else {
                            self.fading_out = None;
                        }
                    }
                } else {
                    // Station had no playable files; stop cleanly.
                    self.fading_out = None;
                    self.active_station = None;
                }
            }
        }
    }

    /// Skips to the next shuffled track in the active station.
    pub fn skip_track(&mut self) -> bool {
        let Some(station) = self.active_station else {
            return false;
        };
        let crossfade = self.crossfade_frames();
        let old = self.current.take();
        if self.start_next_track(station, crossfade > 0 && old.is_some()) {
            if let Some(mut prev) = old
                && crossfade > 0
            {
                prev.start_fade(0, crossfade, true);
                self.fading_out = Some(prev);
            }
            true
        } else {
            false
        }
    }

    /// Stops all music immediately and resets mode to [`MusicMode::None`].
    pub fn stop(&mut self) {
        self.mode = MusicMode::None;
        self.active_station = None;
        self.current = None;
        self.fading_out = None;
    }

    /// Renders `frames` stereo interleaved 16-bit PCM frames (`frames * 2` samples).
    pub fn render_stereo(&mut self, frames: usize) -> Vec<i16> {
        let mut out = vec![0i16; frames.saturating_mul(2)];
        self.mix_into_stereo(&mut out);
        out
    }

    /// Mixes streaming mode music into an interleaved stereo buffer using
    /// saturating addition (so callers can overlay directly onto `Player` output).
    pub fn mix_into_stereo(&mut self, dst: &mut [i16]) {
        if self.paused || dst.len() < 2 {
            return;
        }
        // Combined music + master gain (`0..=1024`).
        let group_gain = ((i64::from(self.volume) * i64::from(self.master_volume)) >> 10)
            .clamp(0, i64::from(MAX_VOLUME));

        for pair in dst.as_chunks_mut::<2>().0 {
            let mut left = 0i32;
            let mut right = 0i32;

            if let Some(deck) = self.current.as_mut() {
                if let Some([l, r]) = deck.next_frame() {
                    left = left.saturating_add(l);
                    right = right.saturating_add(r);
                }
                if deck.is_finished() {
                    let was_stopping = deck.stop_at_zero;
                    self.current = None;
                    if !was_stopping && let Some(station) = self.active_station {
                        self.start_next_track(station, false);
                    }
                }
            }

            if let Some(deck) = self.fading_out.as_mut() {
                if let Some([l, r]) = deck.next_frame() {
                    left = left.saturating_add(l);
                    right = right.saturating_add(r);
                }
                if deck.is_finished() {
                    self.fading_out = None;
                }
            }

            if group_gain > 0 {
                let sl = ((i64::from(left) * group_gain) >> 10)
                    .clamp(i64::from(i16::MIN), i64::from(i16::MAX))
                    as i16;
                let sr = ((i64::from(right) * group_gain) >> 10)
                    .clamp(i64::from(i16::MIN), i64::from(i16::MAX))
                    as i16;
                pair[0] = pair[0].saturating_add(sl);
                pair[1] = pair[1].saturating_add(sr);
            }
        }
    }

    fn crossfade_frames(&self) -> u32 {
        ((u64::from(self.output_rate) * u64::from(self.crossfade_ms)) / 1000) as u32
    }

    /// Number of output frames for `FadeAndKillTrackSlow` (`1024 / 3` ≈ 342
    /// 20 ms ticks = 6.84 s).
    fn slow_fade_frames(&self) -> u32 {
        let ticks = (MAX_VOLUME + SLOW_FADE_STEP_PER_TICK - 1) / SLOW_FADE_STEP_PER_TICK;
        ((u64::from(self.output_rate) * ticks as u64) / u64::from(HIT_TICK_HZ)) as u32
    }

    fn start_next_track(&mut self, mode: MusicMode, fade_in: bool) -> bool {
        let count = self.catalog.tracks(mode).len();
        if count == 0 {
            return false;
        }
        // Try up to `count` candidates so a single corrupt/missing file skips
        // cleanly to the next track instead of stalling playback.
        for _ in 0..count {
            let idx = self.next_playlist_index(mode, count);
            let path = self.catalog.tracks(mode)[idx].clone();
            let Some(bytes) = self.source.read_bytes(&path) else {
                self.faults.push(format!("missing music track: {path}"));
                continue;
            };
            match StreamDeck::open(path.clone(), bytes, self.output_rate) {
                Ok(mut deck) => {
                    if fade_in {
                        let cf = self.crossfade_frames();
                        if cf > 0 {
                            deck.gain_fp = 0;
                            deck.start_fade(MAX_VOLUME, cf, false);
                        }
                    }
                    self.current = Some(deck);
                    return true;
                }
                Err(err) => {
                    self.faults.push(format!("failed to open {path}: {err}"));
                }
            }
        }
        false
    }

    /// Picks the next track index from `mode`'s shuffled cycle, guaranteeing no
    /// immediate repeat whenever `count >= 2`.
    fn next_playlist_index(&mut self, mode: MusicMode, count: usize) -> usize {
        if count <= 1 {
            return 0;
        }
        let state = self.playlists.entry(mode).or_default();
        if state.queue.is_empty() {
            let mut order: Vec<usize> = (0..count).collect();
            // Fisher-Yates shuffle using our deterministic xorshift64* PRNG.
            for i in (1..order.len()).rev() {
                let j = xorshift(&mut self.rng, (i + 1) as u32) as usize;
                order.swap(i, j);
            }
            // Avoid repeating the exact track that just finished (queue pops
            // from the back, so check the last element).
            if let Some(last) = state.last_played
                && order.last().copied() == Some(last)
            {
                let last_idx = order.len() - 1;
                order.swap(0, last_idx);
            }
            state.queue = order;
        }
        let next = state.queue.pop().unwrap_or(0);
        state.last_played = Some(next);
        next
    }
}

fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)).max(1)
}

fn xorshift(state: &mut u64, below: u32) -> u32 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    let value = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
    ((value >> 32) % u64::from(below.max(1))) as u32
}
