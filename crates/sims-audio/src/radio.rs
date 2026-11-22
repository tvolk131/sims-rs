//! In-lot stereo radio and television streaming station playback.
//!
//! Complete Collection 1.91 registers every radio station under
//! `Music\Stations\*` and every television channel under
//! `SoundData\TVStations\*` and `ExpansionPack6\Sound\TVStations\*`
//! (see [`crate::stations::STATIONS`], `0x574c37`). Players can also drop
//! custom `.mp3` / `.wav` files into any existing `Music\Stations\*` folder
//! or add custom subdirectories under `Music\Stations\<CustomName>`.
//!
//! This module builds on [`crate::music`]'s incremental `File`-backed MP3 /
//! WAV / XA [`StreamDeck`] decoder and [`crate::placement`] / [`crate::levels`]
//! spatial math to provide:
//!
//! - [`RadioCatalog`]: deterministic discovery of all built-in stereo radio
//!   stations, TV channels, and custom `Music/Stations/*` directories;
//! - [`SpatialAttenuation`]: screen-space 1.91 projection or tile-distance +
//!   floor attenuation and stereo panning for in-lot stereos and TVs;
//! - [`RadioPlayer`]: multi-emitter streaming mixer with per-station
//!   non-repeating deterministic shuffle, smooth station crossfades, and
//!   separate Music (`kGroupMusic`) vs FX (`kGroupFx`) volume steps matching
//!   1.91's [`VOLUME_STEPS`] curve.

use crate::files::{SoundFiles, Source, key};
use crate::levels;
use crate::music::{MAX_VOLUME, StreamDeck, TrackSource, splitmix64, xorshift};
use crate::placement::{self, CENTRE, FULL, Projection};
use crate::player::{RATE, VOLUME_STEPS};
use std::collections::BTreeMap;

/// Default crossfade duration in milliseconds when switching stations on an
/// active stereo or television emitter.
pub const DEFAULT_STATION_CROSSFADE_MS: u32 = 150;

/// Whether a station belongs to a stereo (Music volume group) or a television
/// (FX volume group).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StationKind {
    /// Stereo / jukebox radio station (`Music\Stations\*`, scaled by Music volume).
    Stereo,
    /// Television channel (`SoundData\TVStations\*` / `ExpansionPack6\Sound\TVStations\*`,
    /// scaled by FX volume).
    Tv,
}

/// Every built-in stereo radio station and television channel in Complete
/// Collection 1.91 (`crate::stations::STATIONS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuiltinStation {
    // Stereo radio stations (`Music\Stations\*`)
    Country,
    Rock,
    Latin,
    Classical,
    CountryDance,
    Rap,
    Rave,
    Disco,
    Beach,
    EasyListening,
    EasyListeningExp,
    UnleashedCheap,
    UnleashedSpeakeasy,
    Superstar,
    // Television channels (`SoundData\TVStations\*` & `ExpansionPack6\Sound\TVStations\*`)
    TvRomance,
    TvAction,
    TvComedy,
    TvMystery,
    TvAnimal,
    TvEntertainment,
    TvNews,
    TvSports,
}

impl BuiltinStation {
    /// Every built-in stereo radio station in registration order.
    pub const STEREO: [Self; 14] = [
        Self::Country,
        Self::Rock,
        Self::Latin,
        Self::Classical,
        Self::CountryDance,
        Self::Rap,
        Self::Rave,
        Self::Disco,
        Self::Beach,
        Self::EasyListening,
        Self::EasyListeningExp,
        Self::UnleashedCheap,
        Self::UnleashedSpeakeasy,
        Self::Superstar,
    ];

    /// Every built-in television channel in registration order.
    pub const TV: [Self; 8] = [
        Self::TvRomance,
        Self::TvAction,
        Self::TvComedy,
        Self::TvMystery,
        Self::TvAnimal,
        Self::TvEntertainment,
        Self::TvNews,
        Self::TvSports,
    ];

    /// Every built-in radio and TV station.
    pub const ALL: [Self; 22] = [
        Self::Country,
        Self::Rock,
        Self::Latin,
        Self::Classical,
        Self::CountryDance,
        Self::Rap,
        Self::Rave,
        Self::Disco,
        Self::Beach,
        Self::EasyListening,
        Self::EasyListeningExp,
        Self::UnleashedCheap,
        Self::UnleashedSpeakeasy,
        Self::Superstar,
        Self::TvRomance,
        Self::TvAction,
        Self::TvComedy,
        Self::TvMystery,
        Self::TvAnimal,
        Self::TvEntertainment,
        Self::TvNews,
        Self::TvSports,
    ];

    /// Whether this station is a stereo radio station or a television channel.
    pub const fn kind(self) -> StationKind {
        match self {
            Self::Country
            | Self::Rock
            | Self::Latin
            | Self::Classical
            | Self::CountryDance
            | Self::Rap
            | Self::Rave
            | Self::Disco
            | Self::Beach
            | Self::EasyListening
            | Self::EasyListeningExp
            | Self::UnleashedCheap
            | Self::UnleashedSpeakeasy
            | Self::Superstar => StationKind::Stereo,
            Self::TvRomance
            | Self::TvAction
            | Self::TvComedy
            | Self::TvMystery
            | Self::TvAnimal
            | Self::TvEntertainment
            | Self::TvNews
            | Self::TvSports => StationKind::Tv,
        }
    }

    /// The 1.91 HIT list number registered for this station (`stations::STATIONS`).
    pub const fn hit_list(self) -> u32 {
        match self {
            Self::Country => 232,
            Self::Rock => 233,
            Self::Latin => 234,
            Self::Classical => 235,
            Self::CountryDance => 1_586,
            Self::Rap => 1_587,
            Self::Rave => 1_588,
            Self::Disco => 1_589,
            Self::Beach => 1_590,
            Self::EasyListening => 65_536,
            Self::EasyListeningExp => 65_537,
            Self::UnleashedCheap => 20_900,
            Self::UnleashedSpeakeasy => 20_901,
            Self::Superstar => 100_100,
            Self::TvRomance => 528,
            Self::TvAction => 529,
            Self::TvComedy => 530,
            Self::TvMystery => 531,
            Self::TvAnimal => 103_000,
            Self::TvEntertainment => 103_001,
            Self::TvNews => 103_002,
            Self::TvSports => 103_003,
        }
    }

    /// Install-relative directory supplying this station's audio tracks.
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Country => "Music\\Stations\\Country",
            Self::Rock => "Music\\Stations\\Rock",
            Self::Latin => "Music\\Stations\\Latin",
            Self::Classical => "Music\\Stations\\Classica",
            Self::CountryDance => "Music\\Stations\\CountryD",
            Self::Rap => "Music\\Stations\\Rap",
            Self::Rave => "Music\\Stations\\Rave",
            Self::Disco => "Music\\Stations\\Disco",
            Self::Beach => "Music\\Stations\\Beach",
            Self::EasyListening => "Music\\Stations\\EZ",
            Self::EasyListeningExp => "Music\\Stations\\EZExp",
            Self::UnleashedCheap => "Music\\Stations\\UnlCheap",
            Self::UnleashedSpeakeasy => "Music\\Stations\\UnlSpeak",
            Self::Superstar => "Music\\Stations\\Superstar",
            Self::TvRomance => "SoundData\\TVStations\\TV_Romance",
            Self::TvAction => "SoundData\\TVStations\\TV_Action",
            Self::TvComedy => "SoundData\\TVStations\\TV_Comedy_Cartoon",
            Self::TvMystery => "SoundData\\TVStations\\TV_Mystery",
            Self::TvAnimal => "ExpansionPack6\\Sound\\TVStations\\Animal",
            Self::TvEntertainment => "ExpansionPack6\\Sound\\TVStations\\Entertainment",
            Self::TvNews => "ExpansionPack6\\Sound\\TVStations\\News",
            Self::TvSports => "ExpansionPack6\\Sound\\TVStations\\Sports",
        }
    }

    /// Short display label for this station.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Country => "Country",
            Self::Rock => "Rock",
            Self::Latin => "Latin",
            Self::Classical => "Classical",
            Self::CountryDance => "Country Dance",
            Self::Rap => "Rap",
            Self::Rave => "Rave",
            Self::Disco => "Disco",
            Self::Beach => "Beach",
            Self::EasyListening => "Easy Listening",
            Self::EasyListeningExp => "Easy Listening (Downtown)",
            Self::UnleashedCheap => "Old Town Band",
            Self::UnleashedSpeakeasy => "Old Town Zydeco",
            Self::Superstar => "Superstar",
            Self::TvRomance => "Romance TV",
            Self::TvAction => "Action TV",
            Self::TvComedy => "Comedy TV",
            Self::TvMystery => "Horror / Mystery TV",
            Self::TvAnimal => "Animal Channel",
            Self::TvEntertainment => "Entertainment Channel",
            Self::TvNews => "News Channel",
            Self::TvSports => "Sports Channel",
        }
    }

    /// Looks up a built-in station by its 1.91 HIT list ID.
    pub fn from_hit_list(hit_list: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.hit_list() == hit_list)
    }

    /// Looks up a built-in stereo station by its `Music\Stations\<dir>` folder
    /// name (ASCII case-insensitive).
    pub fn from_music_folder(folder: &str) -> Option<Self> {
        match folder.to_ascii_lowercase().as_str() {
            "country" => Some(Self::Country),
            "rock" => Some(Self::Rock),
            "latin" => Some(Self::Latin),
            "classica" | "classical" => Some(Self::Classical),
            "countryd" => Some(Self::CountryDance),
            "rap" => Some(Self::Rap),
            "rave" => Some(Self::Rave),
            "disco" => Some(Self::Disco),
            "beach" => Some(Self::Beach),
            "ez" => Some(Self::EasyListening),
            "ezexp" => Some(Self::EasyListeningExp),
            "unlcheap" => Some(Self::UnleashedCheap),
            "unlspeak" => Some(Self::UnleashedSpeakeasy),
            "superstar" => Some(Self::Superstar),
            _ => None,
        }
    }
}

/// Identifier for either a built-in Complete Collection station or a custom
/// station directory placed under `Music/Stations/<name>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StationId {
    Builtin(BuiltinStation),
    /// Custom station subdirectory under `Music/Stations/<name>` (stored in
    /// ASCII lowercase for deterministic ordering and case-insensitive lookup).
    Custom(String),
}

impl StationId {
    /// Creates a normalized custom station identifier.
    pub fn custom(name: impl AsRef<str>) -> Self {
        Self::Custom(name.as_ref().trim().to_ascii_lowercase())
    }

    /// Returns whether this station is a stereo radio station or TV channel.
    pub fn kind(&self) -> StationKind {
        match self {
            Self::Builtin(b) => b.kind(),
            Self::Custom(_) => StationKind::Stereo,
        }
    }

    /// Returns a human-readable display name for this station.
    pub fn label(&self) -> &str {
        match self {
            Self::Builtin(b) => b.label(),
            Self::Custom(name) => name.as_str(),
        }
    }
}

impl From<BuiltinStation> for StationId {
    fn from(station: BuiltinStation) -> Self {
        Self::Builtin(station)
    }
}

/// Discovered track lists for every stereo radio station, TV channel, and
/// custom `Music/Stations/*` directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RadioCatalog {
    tracks: BTreeMap<StationId, Vec<String>>,
}

impl RadioCatalog {
    /// Scans `files` for all loose audio files (`.mp3`, `.wav`, `.xa`) inside:
    /// - every `Music\Stations\<station>\...` folder (built-in and custom),
    /// - every `SoundData\TVStations\<channel>\...` folder (including `a..e`
    ///   subdirectories), and
    /// - every `ExpansionPack6\Sound\TVStations\<channel>\...` folder.
    ///
    /// Tracks within each station are sorted deterministically by [`key`].
    pub fn from_sound_files(files: &SoundFiles) -> Self {
        const MUSIC_STATIONS_PREFIX: &str = "music\\stations\\";
        let tv_prefixes: Vec<(BuiltinStation, String)> = BuiltinStation::TV
            .into_iter()
            .map(|st| (st, format!("{}\\", key(st.directory()))))
            .collect();

        let mut tracks: BTreeMap<StationId, Vec<String>> = BTreeMap::new();

        for entry in files.iter() {
            if !matches!(entry.source, Source::Loose(_)) {
                continue;
            }
            let ext = entry.extension();
            if !matches!(ext.as_str(), "mp3" | "wav" | "xa") {
                continue;
            }
            let normalized = key(&entry.path);

            if let Some(rest) = normalized.strip_prefix(MUSIC_STATIONS_PREFIX) {
                let Some((folder, _)) = rest.split_once('\\') else {
                    continue;
                };
                if folder.is_empty() {
                    continue;
                }
                let station = match BuiltinStation::from_music_folder(folder) {
                    Some(b) => StationId::Builtin(b),
                    None => StationId::Custom(folder.to_owned()),
                };
                tracks.entry(station).or_default().push(entry.path.clone());
                continue;
            }

            for (tv_station, prefix) in &tv_prefixes {
                if normalized.starts_with(prefix.as_str()) {
                    tracks
                        .entry(StationId::Builtin(*tv_station))
                        .or_default()
                        .push(entry.path.clone());
                    break;
                }
            }
        }

        for list in tracks.values_mut() {
            list.sort_by_key(|p| key(p));
        }

        Self { tracks }
    }

    /// Creates an empty catalog (for synthetic unit tests).
    pub fn empty() -> Self {
        Self {
            tracks: BTreeMap::new(),
        }
    }

    /// Registers explicit track paths for `station`.
    pub fn insert(&mut self, station: impl Into<StationId>, paths: Vec<String>) {
        if !paths.is_empty() {
            self.tracks.insert(station.into(), paths);
        }
    }

    /// Returns the discovered tracks for `station` in deterministic key order.
    pub fn tracks(&self, station: &StationId) -> &[String] {
        self.tracks.get(station).map_or(&[], Vec::as_slice)
    }

    /// Iterates over every station that has at least one playable track.
    pub fn stations(&self) -> impl Iterator<Item = &StationId> {
        self.tracks.keys()
    }

    /// Iterates over all discovered stereo stations (built-in and custom).
    pub fn stereo_stations(&self) -> impl Iterator<Item = &StationId> {
        self.tracks
            .keys()
            .filter(|st| st.kind() == StationKind::Stereo)
    }

    /// Iterates over all discovered television channels.
    pub fn tv_stations(&self) -> impl Iterator<Item = &StationId> {
        self.tracks.keys().filter(|st| st.kind() == StationKind::Tv)
    }

    /// Looks up a discovered station by its 1.91 HIT list number.
    pub fn station_for_hit_list(&self, hit_list: u32) -> Option<StationId> {
        let builtin = BuiltinStation::from_hit_list(hit_list)?;
        let id = StationId::Builtin(builtin);
        self.tracks.contains_key(&id).then_some(id)
    }

    /// Resolves a station by folder name or display label (case-insensitive),
    /// returning it only if present in this catalog.
    pub fn station_by_name(&self, name: &str) -> Option<StationId> {
        let trimmed = name.trim();
        if let Some(builtin) = BuiltinStation::from_music_folder(trimmed) {
            let id = StationId::Builtin(builtin);
            if self.tracks.contains_key(&id) {
                return Some(id);
            }
        }
        let needle = trimmed.to_ascii_lowercase();
        if let Some(builtin) = BuiltinStation::ALL
            .into_iter()
            .find(|b| b.label().eq_ignore_ascii_case(trimmed))
        {
            let id = StationId::Builtin(builtin);
            if self.tracks.contains_key(&id) {
                return Some(id);
            }
        }
        let custom = StationId::Custom(needle);
        self.tracks.contains_key(&custom).then_some(custom)
    }

    /// Returns the number of discovered stations.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Returns `true` if no stations were discovered.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

/// Spatial volume attenuation and stereo panning for an in-lot radio or TV
/// emitter.
///
/// `volume` is in `0..=1024` (matching 1.91's `extvol` scale, where `1024` is
/// full volume and `0` is silent) and `pan` is in `0..=1024` (`512` = center,
/// `0` = hard left, `1024` = hard right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialAttenuation {
    pub volume: i32,
    pub pan: i32,
}

impl SpatialAttenuation {
    /// Full volume, centered (no distance attenuation).
    pub const FULL: Self = Self {
        volume: FULL,
        pan: CENTRE,
    };

    /// Completely attenuated (silent).
    pub const SILENT: Self = Self {
        volume: 0,
        pan: CENTRE,
    };

    /// Constructs a clamped spatial attenuation from explicit `volume` and `pan`.
    pub const fn new(volume: i32, pan: i32) -> Self {
        Self {
            volume: clamp_i32(volume, 0, FULL),
            pan: clamp_i32(pan, 0, FULL),
        }
    }

    /// Computes spatial attenuation using Complete Collection 1.91's exact
    /// screen-space 3D placement formula ([`crate::placement::project`]):
    /// zoom drop, off-screen fade, stereo pan, and 3/5 other-floor attenuation.
    pub fn from_projection(spl: i32, level: i32, view: &Projection) -> Result<Self, String> {
        let (volume, pan) = placement::project(spl, level, view)?;
        Ok(Self::new(volume, pan))
    }

    /// Computes spatial attenuation from a lot tile distance to the camera
    /// focus, `max_distance_tiles` falloff radius, whether the emitter is on
    /// the currently viewed floor (3/5 attenuation on other floors, matching
    /// 1.91 `0x57326e`), and horizontal `pan` (`0..=1024`).
    pub fn from_distance(
        distance_tiles: f32,
        max_distance_tiles: f32,
        same_floor: bool,
        pan: i32,
    ) -> Self {
        if !distance_tiles.is_finite() || !max_distance_tiles.is_finite() {
            return Self::SILENT;
        }
        let d = distance_tiles.max(0.0);
        let max_d = max_distance_tiles.max(0.001);
        let factor = (1.0 - (d / max_d)).clamp(0.0, 1.0);
        let mut vol = (factor * FULL as f32).round() as i32;
        if !same_floor {
            vol = (vol * 3) / 5;
        }
        Self::new(vol, pan)
    }
}

impl Default for SpatialAttenuation {
    fn default() -> Self {
        Self::FULL
    }
}

const fn clamp_i32(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[derive(Debug, Clone, Default)]
struct PlaylistState {
    queue: Vec<usize>,
    last_played: Option<usize>,
}

struct EmitterChannel {
    station: StationId,
    spatial: SpatialAttenuation,
    current: Option<StreamDeck>,
    fading_out: Option<StreamDeck>,
}

/// Streaming multi-emitter player for in-lot stereo radio stations and TV
/// channels.
pub struct RadioPlayer<S> {
    catalog: RadioCatalog,
    source: S,
    output_rate: u32,
    emitters: BTreeMap<u32, EmitterChannel>,
    playlists: BTreeMap<StationId, PlaylistState>,
    music_volume_step: u8,
    music_volume: i32,
    fx_volume_step: u8,
    fx_volume: i32,
    master_volume: i32,
    crossfade_ms: u32,
    paused: bool,
    rng: u64,
    /// Non-fatal file/decode diagnostics.
    pub faults: Vec<String>,
}

impl<S: TrackSource> RadioPlayer<S> {
    /// Creates a new radio player at [`RATE`] (22,050 Hz) with deterministic
    /// PRNG `seed`.
    pub fn new(catalog: RadioCatalog, source: S, seed: u64) -> Self {
        Self::with_rate(catalog, source, RATE, seed)
    }

    /// Creates a new radio player targeting `output_rate` Hz.
    pub fn with_rate(catalog: RadioCatalog, source: S, output_rate: u32, seed: u64) -> Self {
        Self {
            catalog,
            source,
            output_rate: output_rate.max(1),
            emitters: BTreeMap::new(),
            playlists: BTreeMap::new(),
            music_volume_step: 10,
            music_volume: MAX_VOLUME,
            fx_volume_step: 10,
            fx_volume: MAX_VOLUME,
            master_volume: MAX_VOLUME,
            crossfade_ms: DEFAULT_STATION_CROSSFADE_MS,
            paused: false,
            rng: splitmix64(seed),
            faults: Vec::new(),
        }
    }

    pub fn catalog(&self) -> &RadioCatalog {
        &self.catalog
    }

    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }

    /// Sets the crossfade duration in milliseconds when switching stations on
    /// an active emitter (`0` for hard cuts).
    pub fn set_crossfade_ms(&mut self, ms: u32) {
        self.crossfade_ms = ms;
    }

    /// Sets the Music volume slider step (`0..=10`, clamped) used by stereo
    /// radio stations, matching 1.91's [`VOLUME_STEPS`] table.
    pub fn set_music_volume_step(&mut self, step: usize) {
        let clamped = step.min(VOLUME_STEPS.len() - 1);
        self.music_volume_step = clamped as u8;
        self.music_volume = VOLUME_STEPS[clamped];
    }

    pub fn music_volume_step(&self) -> u8 {
        self.music_volume_step
    }

    pub fn set_music_volume(&mut self, volume: i32) {
        self.music_volume = volume.clamp(0, MAX_VOLUME);
    }

    pub fn music_volume(&self) -> i32 {
        self.music_volume
    }

    /// Sets the FX volume slider step (`0..=10`, clamped) used by TV channels,
    /// matching 1.91's [`VOLUME_STEPS`] table.
    pub fn set_fx_volume_step(&mut self, step: usize) {
        let clamped = step.min(VOLUME_STEPS.len() - 1);
        self.fx_volume_step = clamped as u8;
        self.fx_volume = VOLUME_STEPS[clamped];
    }

    pub fn fx_volume_step(&self) -> u8 {
        self.fx_volume_step
    }

    pub fn set_fx_volume(&mut self, volume: i32) {
        self.fx_volume = volume.clamp(0, MAX_VOLUME);
    }

    pub fn fx_volume(&self) -> i32 {
        self.fx_volume
    }

    /// Sets the master volume (`0..=1024`, clamped).
    pub fn set_master_volume(&mut self, volume: i32) {
        self.master_volume = volume.clamp(0, MAX_VOLUME);
    }

    pub fn master_volume(&self) -> i32 {
        self.master_volume
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Number of currently active radio/TV emitters.
    pub fn active_emitters(&self) -> usize {
        self.emitters.len()
    }

    /// Returns the station currently tuned on `emitter_id`.
    pub fn emitter_station(&self, emitter_id: u32) -> Option<&StationId> {
        self.emitters.get(&emitter_id).map(|ch| &ch.station)
    }

    /// Returns the currently playing track path on `emitter_id`.
    pub fn emitter_track(&self, emitter_id: u32) -> Option<&str> {
        self.emitters
            .get(&emitter_id)
            .and_then(|ch| ch.current.as_ref())
            .map(|d| d.path.as_str())
    }

    /// Returns the current spatial attenuation of `emitter_id`.
    pub fn emitter_spatial(&self, emitter_id: u32) -> Option<SpatialAttenuation> {
        self.emitters.get(&emitter_id).map(|ch| ch.spatial)
    }

    /// Tunes `emitter_id` (e.g. a stereo or TV object ID) to `station` with
    /// initial `spatial` attenuation.
    ///
    /// If `emitter_id` is already playing another station and `crossfade_ms > 0`,
    /// the outgoing track crossfades smoothly into the new station. Returns
    /// `true` if a track was opened for `station`.
    pub fn tune(
        &mut self,
        emitter_id: u32,
        station: impl Into<StationId>,
        spatial: SpatialAttenuation,
    ) -> bool {
        let station = station.into();
        let cf = self.crossfade_frames();

        if let Some(existing) = self.emitters.get_mut(&emitter_id) {
            existing.spatial = spatial;
            if existing.station == station && existing.current.is_some() {
                return true;
            }
        }

        let prev_deck = self
            .emitters
            .get_mut(&emitter_id)
            .and_then(|ch| ch.current.take());
        let fade_in = cf > 0 && prev_deck.is_some();

        let Some(new_deck) = self.open_next_track(&station, fade_in) else {
            // Keep nothing active if the requested station has no playable tracks.
            self.emitters.remove(&emitter_id);
            return false;
        };

        let fading_out = prev_deck.and_then(|mut old| {
            if cf > 0 {
                old.start_fade(0, cf, true);
                Some(old)
            } else {
                None
            }
        });

        self.emitters.insert(
            emitter_id,
            EmitterChannel {
                station,
                spatial,
                current: Some(new_deck),
                fading_out,
            },
        );
        true
    }

    /// Updates the spatial attenuation of an active emitter (e.g. when the
    /// camera pans, zooms, or changes floor).
    pub fn set_emitter_spatial(&mut self, emitter_id: u32, spatial: SpatialAttenuation) {
        if let Some(ch) = self.emitters.get_mut(&emitter_id) {
            ch.spatial = spatial;
        }
    }

    /// Updates `emitter_id`'s spatial attenuation from a 1.91 screen-space
    /// [`Projection`].
    pub fn set_emitter_projection(
        &mut self,
        emitter_id: u32,
        spl: i32,
        level: i32,
        view: &Projection,
    ) -> Result<(), String> {
        let spatial = SpatialAttenuation::from_projection(spl, level, view)?;
        self.set_emitter_spatial(emitter_id, spatial);
        Ok(())
    }

    /// Advances `emitter_id` to the next non-repeating shuffled track on its
    /// current station (`radio_next_song`).
    pub fn skip_track(&mut self, emitter_id: u32) -> bool {
        let Some(station) = self.emitters.get(&emitter_id).map(|ch| ch.station.clone()) else {
            return false;
        };
        let cf = self.crossfade_frames();
        let prev = self
            .emitters
            .get_mut(&emitter_id)
            .and_then(|ch| ch.current.take());
        let fade_in = cf > 0 && prev.is_some();
        let Some(new_deck) = self.open_next_track(&station, fade_in) else {
            self.emitters.remove(&emitter_id);
            return false;
        };
        if let Some(ch) = self.emitters.get_mut(&emitter_id) {
            ch.current = Some(new_deck);
            ch.fading_out = prev.and_then(|mut old| {
                if cf > 0 {
                    old.start_fade(0, cf, true);
                    Some(old)
                } else {
                    None
                }
            });
            true
        } else {
            false
        }
    }

    /// Turns off `emitter_id` immediately.
    pub fn stop_emitter(&mut self, emitter_id: u32) {
        self.emitters.remove(&emitter_id);
    }

    /// Stops every active stereo and TV emitter.
    pub fn stop_all(&mut self) {
        self.emitters.clear();
    }

    /// Renders `frames` stereo interleaved 16-bit PCM frames (`frames * 2`
    /// samples).
    pub fn render_stereo(&mut self, frames: usize) -> Vec<i16> {
        let mut out = vec![0i16; frames.saturating_mul(2)];
        self.mix_into_stereo(&mut out);
        out
    }

    /// Mixes all active radio and TV emitters into an interleaved stereo
    /// buffer using saturating addition.
    pub fn mix_into_stereo(&mut self, dst: &mut [i16]) {
        if self.paused || dst.len() < 2 || self.emitters.is_empty() {
            return;
        }

        let emitter_ids: Vec<u32> = self.emitters.keys().copied().collect();

        for pair in dst.as_chunks_mut::<2>().0 {
            let mut sum_l = 0i64;
            let mut sum_r = 0i64;

            for &id in &emitter_ids {
                let mut advance_station: Option<StationId> = None;
                let (mut ch_l, mut ch_r) = (0i32, 0i32);
                let (station_kind, spatial) = {
                    let Some(ch) = self.emitters.get_mut(&id) else {
                        continue;
                    };
                    if let Some(deck) = ch.current.as_mut() {
                        if let Some([l, r]) = deck.next_frame() {
                            ch_l = ch_l.saturating_add(l);
                            ch_r = ch_r.saturating_add(r);
                        }
                        if deck.is_finished() {
                            let was_stopping = deck.stop_at_zero;
                            ch.current = None;
                            if !was_stopping {
                                advance_station = Some(ch.station.clone());
                            }
                        }
                    }
                    if let Some(deck) = ch.fading_out.as_mut() {
                        if let Some([l, r]) = deck.next_frame() {
                            ch_l = ch_l.saturating_add(l);
                            ch_r = ch_r.saturating_add(r);
                        }
                        if deck.is_finished() {
                            ch.fading_out = None;
                        }
                    }
                    (ch.station.kind(), ch.spatial)
                };

                if let Some(station) = advance_station {
                    let next = self.open_next_track(&station, false);
                    if let Some(ch) = self.emitters.get_mut(&id) {
                        ch.current = next;
                    }
                }

                let group_vol = match station_kind {
                    StationKind::Stereo => self.music_volume,
                    StationKind::Tv => self.fx_volume,
                };
                let eff_vol = (i64::from(spatial.volume) * i64::from(group_vol)) / 1024;
                if eff_vol > 0 && self.master_volume > 0 {
                    let att = levels::attenuation(eff_vol, i64::from(self.master_volume));
                    let [gl, gr] = levels::gains(att, levels::pan(spatial.pan));
                    sum_l += (i64::from(ch_l) * i64::from(gl)) >> 16;
                    sum_r += (i64::from(ch_r) * i64::from(gr)) >> 16;
                }
            }

            let sl = sum_l.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
            let sr = sum_r.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
            pair[0] = pair[0].saturating_add(sl);
            pair[1] = pair[1].saturating_add(sr);
        }

        self.emitters
            .retain(|_, ch| ch.current.is_some() || ch.fading_out.is_some());
    }

    fn crossfade_frames(&self) -> u32 {
        ((u64::from(self.output_rate) * u64::from(self.crossfade_ms)) / 1000)
            .min(u64::from(u32::MAX)) as u32
    }

    fn open_next_track(&mut self, station: &StationId, fade_in: bool) -> Option<StreamDeck> {
        let count = self.catalog.tracks(station).len();
        if count == 0 {
            return None;
        }
        let prev_played = self.playlists.get(station).and_then(|p| p.last_played);
        let mut tried = vec![false; count];
        let mut unique_tried = 0usize;
        while unique_tried < count {
            let candidate = self.next_playlist_index(station, count);
            let idx = if !tried[candidate] {
                candidate
            } else if let Some(other) = (0..count).find(|&i| !tried[i] && Some(i) != prev_played) {
                other
            } else if let Some(any) = (0..count).find(|&i| !tried[i]) {
                any
            } else {
                break;
            };
            tried[idx] = true;
            unique_tried += 1;
            let path = self.catalog.tracks(station)[idx].clone();
            let Some(media) = self.source.open_track(&path) else {
                self.faults.push(format!("missing radio track: {path}"));
                continue;
            };
            match StreamDeck::open_media(path.clone(), media, self.output_rate) {
                Ok(mut deck) => {
                    if fade_in {
                        let cf = self.crossfade_frames();
                        if cf > 0 {
                            deck.gain_fp = 0;
                            deck.start_fade(MAX_VOLUME, cf, false);
                        }
                    }
                    if let Some(state) = self.playlists.get_mut(station) {
                        state.last_played = Some(idx);
                    }
                    return Some(deck);
                }
                Err(err) => {
                    self.faults.push(format!("failed to open {path}: {err}"));
                }
            }
        }
        None
    }

    fn next_playlist_index(&mut self, station: &StationId, count: usize) -> usize {
        if count <= 1 {
            return 0;
        }
        let state = self.playlists.entry(station.clone()).or_default();
        if state.queue.is_empty() {
            let mut order: Vec<usize> = (0..count).collect();
            for i in (1..order.len()).rev() {
                let j = xorshift(&mut self.rng, (i + 1) as u32) as usize;
                order.swap(i, j);
            }
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
