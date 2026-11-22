//! Simlish voice bank index, deterministic resolver, and pitch-shifted PCM
//! streaming over Complete Collection's voice directories.
//!
//! Discovers Simlish voice clips (`.XA` / `.WAV`) and voice script sets
//! (`.HIT` / `.HOT`) across:
//!
//! - `SoundData\Voice\*` and `SoundData\VOX\*` (base game `SoundData.far` and
//!   loose overrides, plus `SoundData\Phone\Vox_*`),
//! - `ExpansionPack*\Sound\vox\*`, `voxcat\*`, `voxdog\*`, `npc\*`,
//! - `ExpansionShared\Sound\Vox\*` and `ExpansionShared\Sound\*_vox*.xa`,
//! - `ExpansionPack\*_vox*.xa` and voice `.hit`/`.hot` modules (e.g.
//!   `ExpansionPack\repair_vox2.hit`).
//!
//! Every clip is classified by [`VoiceAgeGender`] (Adult Male, Adult Female,
//! Child, Baby, Dog, Cat, NPC, or Unspecified), grouped by normalized category,
//! and decoded through the existing [`crate::decode`] (`xa` / `wav`) decoders.

use crate::files::{Entry, SoundFiles, key};
use crate::levels::{BASE_PITCH, BASE_RATE, rate as pitch_to_rate};
use crate::{Error, Pcm, Result, decode};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Voice age, gender, species, or NPC classification for a Simlish voice clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VoiceAgeGender {
    AdultMale,
    AdultFemale,
    Child,
    Baby,
    Dog,
    Cat,
    Npc,
    Unspecified,
}

impl VoiceAgeGender {
    /// All voice bank buckets in canonical order.
    pub const ALL: [Self; 8] = [
        Self::AdultMale,
        Self::AdultFemale,
        Self::Child,
        Self::Baby,
        Self::Dog,
        Self::Cat,
        Self::Npc,
        Self::Unspecified,
    ];
}

/// Pitch setting for Simlish voice playback, matching HIT's cent scale
/// (`BASE_PITCH = 3600` plays at native `22,050 Hz`; `1200` cents = 1 octave).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoicePitch(pub i32);

impl Default for VoicePitch {
    fn default() -> Self {
        Self(BASE_PITCH)
    }
}

impl VoicePitch {
    /// Native unshifted pitch (`3600` cents -> `22,050 Hz`).
    pub const NATIVE: Self = Self(BASE_PITCH);

    /// Constructs a pitch shifted by `semitones` relative to native pitch
    /// (100 cents per semitone, clamped to `[-24.0, +24.0]`).
    pub fn from_semitones(semitones: f32) -> Self {
        let clamped = if semitones.is_finite() {
            semitones.clamp(-24.0, 24.0)
        } else {
            0.0
        };
        Self(BASE_PITCH + (clamped * 100.0).round() as i32)
    }

    /// Returns the HIT pitch value in cents.
    pub const fn cents(self) -> i32 {
        self.0
    }

    /// Playback rate in frames per second from [`crate::levels::rate`].
    pub fn playback_rate(self) -> u32 {
        pitch_to_rate(self.0)
    }
}

/// One indexed Simlish voice audio clip (`.XA` or `.WAV`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceClip {
    /// Lowercase normalized lookup key (`files::key`).
    pub key: String,
    /// Original relative path inside the install (with `\` separators).
    pub path: String,
    /// Top-level install pack or directory (`sounddata`, `expansionpack5`, ...).
    pub pack: String,
    /// Normalized voice category (e.g. `converse`, `laugh`, `baby`, `voxdog`, `npc`).
    pub category: String,
    /// Lowercase filename stem without extension.
    pub stem: String,
    /// Lowercase extension (`"xa"` or `"wav"`).
    pub extension: String,
    /// Classified age/gender/species/NPC bucket.
    pub voice: VoiceAgeGender,
}

/// One discovered voice HIT/HOT script set (e.g. `ExpansionPack\repair_vox2.hit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceScriptSet {
    pub key: String,
    pub path: String,
    pub pack: String,
    pub extension: String,
}

/// Filter criteria for [`VoiceCatalog::search`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoiceQuery<'a> {
    pub voice: Option<VoiceAgeGender>,
    pub category: Option<&'a str>,
    pub name_contains: Option<&'a str>,
    pub pack: Option<&'a str>,
}

/// Indexed Simlish voice bank over an install's [`SoundFiles`].
#[derive(Debug, Clone)]
pub struct VoiceCatalog {
    files: SoundFiles,
    clips: Vec<VoiceClip>,
    by_key: BTreeMap<String, usize>,
    by_voice: BTreeMap<VoiceAgeGender, Vec<usize>>,
    by_category: BTreeMap<String, Vec<usize>>,
    script_sets: Vec<VoiceScriptSet>,
}

impl VoiceCatalog {
    /// Scans `root` for sound files and builds the Simlish voice bank index.
    pub fn scan(root: &Path) -> Result<Self> {
        let files = SoundFiles::scan(root)?;
        Ok(Self::from_sound_files(files))
    }

    /// Builds a voice bank index from an already-scanned [`SoundFiles`] table.
    pub fn from_sound_files(files: SoundFiles) -> Self {
        let mut clips = Vec::new();
        let mut script_sets = Vec::new();

        for entry in files.iter() {
            let ext = entry.extension();
            if !is_voice_entry(&entry.path, &ext) {
                continue;
            }
            let norm = key(&entry.path);
            let pack = top_pack(&norm);
            match ext.as_str() {
                "xa" | "wav" => {
                    let stem = file_stem(&norm);
                    let category = extract_category(&norm, &stem);
                    let voice = classify_voice(&norm, &category, &stem);
                    clips.push(VoiceClip {
                        key: norm,
                        path: entry.path.clone(),
                        pack,
                        category,
                        stem,
                        extension: ext,
                        voice,
                    });
                }
                "hit" | "hot" => {
                    script_sets.push(VoiceScriptSet {
                        key: norm,
                        path: entry.path.clone(),
                        pack,
                        extension: ext,
                    });
                }
                _ => {}
            }
        }

        // Deterministic ordering by normalized key.
        clips.sort_by(|a, b| a.key.cmp(&b.key));
        script_sets.sort_by(|a, b| a.key.cmp(&b.key));

        let mut by_key = BTreeMap::new();
        let mut by_voice: BTreeMap<VoiceAgeGender, Vec<usize>> = BTreeMap::new();
        let mut by_category: BTreeMap<String, Vec<usize>> = BTreeMap::new();

        for (idx, clip) in clips.iter().enumerate() {
            by_key.insert(clip.key.clone(), idx);
            by_voice.entry(clip.voice).or_default().push(idx);
            by_category
                .entry(clip.category.clone())
                .or_default()
                .push(idx);
        }

        Self {
            files,
            clips,
            by_key,
            by_voice,
            by_category,
            script_sets,
        }
    }

    /// Total number of indexed voice audio clips (`.XA` + `.WAV`).
    pub fn len(&self) -> usize {
        self.clips.len()
    }

    /// Whether the catalog contains no voice audio clips.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// All indexed voice clips in deterministic key order.
    pub fn clips(&self) -> &[VoiceClip] {
        &self.clips
    }

    /// All discovered voice `.HIT` / `.HOT` script sets.
    pub fn script_sets(&self) -> &[VoiceScriptSet] {
        &self.script_sets
    }

    /// Underlying [`SoundFiles`] index.
    pub fn sound_files(&self) -> &SoundFiles {
        &self.files
    }

    /// Distinct categories discovered across all voice clips, sorted.
    pub fn categories(&self) -> Vec<&str> {
        self.by_category.keys().map(String::as_str).collect()
    }

    /// Distinct top-level packs containing voice clips, sorted.
    pub fn packs(&self) -> Vec<&str> {
        let packs: BTreeSet<&str> = self.clips.iter().map(|c| c.pack.as_str()).collect();
        packs.into_iter().collect()
    }

    /// Number of voice clips classified under `voice`.
    pub fn count_for(&self, voice: VoiceAgeGender) -> usize {
        self.by_voice.get(&voice).map_or(0, Vec::len)
    }

    /// All voice clips classified under `voice` in deterministic order.
    pub fn clips_for(&self, voice: VoiceAgeGender) -> Vec<&VoiceClip> {
        self.by_voice
            .get(&voice)
            .map(|indices| indices.iter().map(|&i| &self.clips[i]).collect())
            .unwrap_or_default()
    }

    /// Looks up a single clip by relative install path (case/slash-insensitive).
    pub fn resolve_path(&self, path: &str) -> Option<&VoiceClip> {
        let norm = key(path);
        self.by_key.get(&norm).map(|&idx| &self.clips[idx])
    }

    /// Searches clips matching every specified field in `query`.
    pub fn search(&self, query: &VoiceQuery<'_>) -> Vec<&VoiceClip> {
        let cat_lower = query.category.map(str::to_ascii_lowercase);
        let name_lower = query.name_contains.map(str::to_ascii_lowercase);
        let pack_lower = query.pack.map(str::to_ascii_lowercase);

        let base_indices: Option<&[usize]> = match (&query.voice, &cat_lower) {
            (Some(v), _) => self.by_voice.get(v).map(Vec::as_slice),
            (None, Some(c)) => self.by_category.get(c).map(Vec::as_slice),
            (None, None) => None,
        };

        let matches = |clip: &VoiceClip| -> bool {
            if let Some(v) = query.voice
                && clip.voice != v
            {
                return false;
            }
            if let Some(ref c) = cat_lower
                && clip.category != *c
            {
                return false;
            }
            if let Some(ref n) = name_lower
                && !clip.stem.contains(n.as_str())
                && !clip.key.contains(n.as_str())
            {
                return false;
            }
            if let Some(ref p) = pack_lower
                && clip.pack != *p
            {
                return false;
            }
            true
        };

        match base_indices {
            Some(indices) => indices
                .iter()
                .map(|&i| &self.clips[i])
                .filter(|c| matches(c))
                .collect(),
            None => self.clips.iter().filter(|c| matches(c)).collect(),
        }
    }

    /// Deterministically picks one clip matching `voice` and optional `category`
    /// using `seed` (`matches[seed % len]`).
    pub fn pick(
        &self,
        voice: VoiceAgeGender,
        category: Option<&str>,
        seed: u64,
    ) -> Option<&VoiceClip> {
        let matches = self.search(&VoiceQuery {
            voice: Some(voice),
            category,
            ..VoiceQuery::default()
        });
        if matches.is_empty() {
            return None;
        }
        let idx = (seed as usize) % matches.len();
        Some(matches[idx])
    }

    /// Decodes a voice clip to its native [`Pcm`] using the crate's XA/WAV
    /// decoders.
    pub fn decode_clip(&self, clip: &VoiceClip) -> Result<Pcm> {
        let entry: &Entry = self.files.get(&clip.key).ok_or_else(|| {
            Error::format(
                "VOX",
                0,
                format!("voice clip not in SoundFiles: {}", clip.path),
            )
        })?;
        let bytes = entry.read()?;
        decode::decode(&bytes).ok_or_else(|| {
            Error::format(
                "VOX",
                0,
                format!("unrecognized audio header: {}", clip.path),
            )
        })?
    }

    /// Decodes `clip` and resamples it to `output_rate` (default [`BASE_RATE`])
    /// at `pitch`, using deterministic linear interpolation.
    pub fn decode_pitched(
        &self,
        clip: &VoiceClip,
        pitch: VoicePitch,
        output_rate: u32,
    ) -> Result<Pcm> {
        let pcm = self.decode_clip(clip)?;
        Ok(resample_pitched(&pcm, pitch, output_rate))
    }

    /// Opens an incremental [`VoiceStream`] for `clip` at `pitch` and
    /// `output_rate`.
    pub fn open_stream(
        &self,
        clip: &VoiceClip,
        pitch: VoicePitch,
        output_rate: u32,
    ) -> Result<VoiceStream> {
        let pcm = self.decode_pitched(clip, pitch, output_rate)?;
        Ok(VoiceStream::new(pcm))
    }
}

/// Incremental PCM reader over a decoded (and optionally pitch-shifted) voice
/// clip, suitable for feeding an audio mixer callback without reallocating.
#[derive(Debug, Clone)]
pub struct VoiceStream {
    pcm: Pcm,
    cursor_frame: usize,
}

impl VoiceStream {
    pub fn new(pcm: Pcm) -> Self {
        Self {
            pcm,
            cursor_frame: 0,
        }
    }

    pub fn rate(&self) -> u32 {
        self.pcm.rate
    }

    pub fn channels(&self) -> u16 {
        self.pcm.channels
    }

    pub fn total_frames(&self) -> usize {
        self.pcm.frames()
    }

    pub fn remaining_frames(&self) -> usize {
        self.total_frames().saturating_sub(self.cursor_frame)
    }

    pub fn is_finished(&self) -> bool {
        self.remaining_frames() == 0
    }

    pub fn reset(&mut self) {
        self.cursor_frame = 0;
    }

    /// Normalized playback progress in `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        let total = self.total_frames();
        if total == 0 {
            1.0
        } else {
            (self.cursor_frame as f32 / total as f32).clamp(0.0, 1.0)
        }
    }

    /// Copies up to `out.len() / channels` interleaved frames into `out` and
    /// returns the number of frames written.
    pub fn read_frames(&mut self, out: &mut [i16]) -> usize {
        let ch = usize::from(self.pcm.channels);
        if ch == 0 {
            return 0;
        }
        let max_frames = out.len() / ch;
        let to_read = max_frames.min(self.remaining_frames());
        if to_read == 0 {
            return 0;
        }
        let start = self.cursor_frame * ch;
        let count = to_read * ch;
        out[..count].copy_from_slice(&self.pcm.samples[start..start + count]);
        self.cursor_frame += to_read;
        to_read
    }
}

/// Deterministically resamples `pcm` according to `pitch` (relative to
/// [`BASE_RATE`]) and target `output_rate`.
pub fn resample_pitched(pcm: &Pcm, pitch: VoicePitch, output_rate: u32) -> Pcm {
    let ch = usize::from(pcm.channels);
    let in_frames = pcm.frames();
    let target_rate = output_rate.max(1);
    if ch == 0 || in_frames == 0 || pcm.rate == 0 {
        return Pcm {
            rate: target_rate,
            channels: pcm.channels,
            samples: Vec::new(),
        };
    }

    // Effective source playback rate: scale sample's native rate by pitch ratio.
    let pitch_rate = f64::from(pitch.playback_rate());
    let effective_src_rate = f64::from(pcm.rate) * (pitch_rate / f64::from(BASE_RATE));
    let step = effective_src_rate / f64::from(target_rate);

    if (step - 1.0).abs() < 1e-9 && pcm.rate == target_rate {
        return pcm.clone();
    }

    let out_frames = ((in_frames as f64) / step).ceil().max(1.0) as usize;
    let mut samples = Vec::with_capacity(out_frames * ch);
    for f in 0..out_frames {
        let pos = f as f64 * step;
        let i0 = (pos.floor() as usize).min(in_frames - 1);
        let i1 = (i0 + 1).min(in_frames - 1);
        let frac = pos - (i0 as f64);
        for c in 0..ch {
            let s0 = f64::from(pcm.samples[i0 * ch + c]);
            let s1 = f64::from(pcm.samples[i1 * ch + c]);
            let val = (s0 + (s1 - s0) * frac)
                .round()
                .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
            samples.push(val);
        }
    }

    Pcm {
        rate: target_rate,
        channels: pcm.channels,
        samples,
    }
}

/// Returns whether a sound file path belongs to Complete Collection's Simlish
/// voice banks.
fn is_voice_entry(path: &str, ext: &str) -> bool {
    if !matches!(ext, "xa" | "wav" | "hit" | "hot") {
        return false;
    }
    let norm = key(path);
    let parts: Vec<&str> = norm.split('\\').collect();
    let filename = parts.last().copied().unwrap_or_default();

    // Directory-based voice roots:
    // - SoundData\Voice\* or SoundData\VOX\*
    // - ExpansionPack*\Sound\vox\*, voxcat\*, voxdog\*, npc\*
    // - ExpansionShared\Sound\Vox\*
    for dir in &parts[..parts.len().saturating_sub(1)] {
        if matches!(*dir, "voice" | "vox" | "voxcat" | "voxdog" | "npc") {
            return true;
        }
    }

    // Filename-based voice entries in SoundData\Phone, ExpansionPack, ExpansionShared:
    if filename.contains("_vox")
        || filename.starts_with("vox_")
        || filename.starts_with("voxcat_")
        || filename.starts_with("voxdog_")
        || filename.contains("voxf")
        || filename.contains("voxm")
        || filename.contains("voxk")
    {
        return true;
    }

    false
}

fn top_pack(norm: &str) -> String {
    norm.split('\\').next().unwrap_or("unknown").to_owned()
}

fn file_stem(norm: &str) -> String {
    let name = norm.rsplit('\\').next().unwrap_or_default();
    match name.rsplit_once('.') {
        Some((stem, _)) => stem.to_owned(),
        None => name.to_owned(),
    }
}

/// Extracts a normalized voice category from directory structure or filename prefix.
fn extract_category(norm: &str, stem: &str) -> String {
    let parts: Vec<&str> = norm.split('\\').collect();
    // SoundData\Voice\<category>\<clip>.xa
    if let Some(pos) = parts.iter().position(|&p| p == "voice")
        && let Some(&sub) = parts.get(pos + 1)
        && pos + 2 < parts.len()
    {
        return sub.to_owned();
    }
    // ExpansionPack5\Sound\voxcat\<clip>.xa or voxdog or npc
    for &dir in &parts[..parts.len().saturating_sub(1)] {
        if matches!(dir, "voxcat" | "voxdog" | "npc" | "phone") {
            return dir.to_owned();
        }
    }
    // Otherwise derive category from leading stem token (e.g. "mm_golf_celebrate_voxf1" -> "mm_golf").
    let stripped = stem.strip_prefix("vox_").unwrap_or(stem);
    stripped
        .split('_')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("general")
        .to_owned()
}

/// Classifies a voice clip into [`VoiceAgeGender`] using Complete Collection's
/// directory and suffix conventions.
fn classify_voice(norm: &str, category: &str, stem: &str) -> VoiceAgeGender {
    // 1. Species & baby directories first (highest specificity).
    if category == "voxdog" || stem.starts_with("voxdog") || stem.contains("_dog_") {
        return VoiceAgeGender::Dog;
    }
    if category == "voxcat" || stem.starts_with("voxcat") || stem.contains("_cat_") {
        return VoiceAgeGender::Cat;
    }
    if category == "baby" || stem.starts_with("baby_") {
        // Note: `VOX_BABYTALKF1.XA` is an adult talking to a baby; check suffix first.
        if stem.starts_with("baby_") {
            return VoiceAgeGender::Baby;
        }
    }

    // 2. Explicit NPC directories or known NPC filename prefixes.
    if category == "npc"
        || category == "ghost"
        || norm.contains("\\npc\\")
        || stem.starts_with("npc_")
        || stem.starts_with("nfass_")
        || stem.starts_with("nfmas_")
        || stem.starts_with("nmmas_")
        || stem.starts_with("nmpap_")
        || stem.starts_with("nffla_")
        || stem.starts_with("barmaid_")
        || stem.starts_with("bart_")
        || stem.starts_with("clown_")
        || stem.starts_with("beehive_bear")
        || stem.starts_with("dragon_")
        || stem.contains("voxnf")
        || stem.contains("voxnm")
    {
        return VoiceAgeGender::Npc;
    }

    // 3. Standard Simlish voice suffix before trailing digits:
    //    `...f12` -> AdultFemale, `...m3` -> AdultMale, `...k8` -> Child.
    let without_digits = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    if without_digits.len() < stem.len()
        && let Some(last) = without_digits.chars().last()
    {
        match last {
            'f' if is_voice_gender_marker(without_digits, 'f') => {
                return VoiceAgeGender::AdultFemale;
            }
            'm' if is_voice_gender_marker(without_digits, 'm') => {
                return VoiceAgeGender::AdultMale;
            }
            'k' if is_voice_gender_marker(without_digits, 'k') => {
                return VoiceAgeGender::Child;
            }
            _ => {}
        }
    }

    // 4. Explicit `_f_`, `_m_`, `_k_`, `child`, `female`, `male` tokens.
    if stem.contains("_female") || stem.contains("voxf") {
        return VoiceAgeGender::AdultFemale;
    }
    if stem.contains("_male") || stem.contains("voxm") {
        return VoiceAgeGender::AdultMale;
    }
    if stem.contains("_child") || stem.contains("voxk") {
        return VoiceAgeGender::Child;
    }

    VoiceAgeGender::Unspecified
}

/// Guards against misclassifying words that happen to end in `f`/`m`/`k` when
/// preceded by unrelated suffixes, while accepting Complete Collection's
/// `...voxF1`, `...talkF2`, `...highK3`, `...arribaM1`, `...cryF4`, etc.
fn is_voice_gender_marker(without_digits: &str, marker: char) -> bool {
    let prefix = &without_digits[..without_digits.len() - marker.len_utf8()];
    // Reject obvious non-marker stems like "vox" itself.
    !prefix.is_empty() && !matches!(without_digits, "vox" | "sfx" | "bgm")
}
