//! The animation clips a person's animator can play, and what of each one
//! the simulation reads.
//!
//! The original looks a clip up by name in its animation library
//! (0x59dc32, through 0x5a03e4) and builds a layer from it (0x597a88). Of
//! the clip the layer keeps its duration (clip +0x2c), the frame count of
//! its first motion (layer +0x38), and one binding per motion (layer +8),
//! each with a cursor over the motion's timed events ([`Track`]). The samples (the
//! `.cfp` file named after the clip's skill) pose the skeleton. The
//! simulation reads a pose only where the person's constructor measures
//! the walk's strides (0x58831d); the rest is presentation. A clip whose
//! samples are missing is still refused, since how the original copes
//! with that is not ported.

use crate::index::{all_far_members, sorted_entries};
use crate::install::resolve;
use crate::{Error, Index, Install, Resource, Result};
use sims_formats::animation::{Samples, read_bcf, read_cfp};
use std::collections::{BTreeMap, BTreeSet};

/// What the animator reads of one clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clip {
    /// The clip's name, as the file spells it.
    pub name: String,
    /// The skill: the name of the file that holds the clip's samples.
    pub skill: String,
    /// The clip's duration in milliseconds, as the bits of the file's
    /// single-precision float (clip +0x2c).
    pub duration: u32,
    /// How far the clip moves the person, in tiles, as the bits of the
    /// file's single-precision float (clip +0x30, read by the animator's
    /// virtual function 0x120, 0x588ed7).
    pub distance: u32,
    /// The file's flag after the distance (clip +0x34, read at 0x59703b):
    /// the clip moves the person. A walk that starts its clip keeps the
    /// phase of the clip playing before only when this is set (0x58edb6).
    pub moving: bool,
    /// The frame count of the clip's first motion, or 0 for a clip
    /// without motions (layer +0x38, 0x597b01).
    pub frames: u32,
    /// How many motions the clip has.
    pub motions: usize,
    /// The timed events of each motion that has any, in the order of the
    /// motions. Each becomes a cursor of the layer's binding for that
    /// motion (0x594778), and the layer update dispatches them (0x597c4c).
    pub tracks: Vec<Track>,
    /// Each motion's samples, in the order of the motions.
    pub channels: Vec<Channel>,
    /// How many positions and rotations the sample file holds for the
    /// clip, which reading it needs.
    pub samples: [u32; 2],
}

/// Where one motion's samples are in its clip's sample file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// The bone the motion moves.
    pub bone: String,
    /// The index of the motion's first position, if it has positions.
    pub translation: Option<u32>,
    /// The index of the motion's first rotation, if it has rotations.
    pub rotation: Option<u32>,
}

/// One motion's timed events, as the original builds its list when it
/// reads the clip (0x5946fc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// The bone the motion moves. Its binding exists only if the person's
    /// skeleton has the bone.
    pub bone: String,
    /// The events, ordered by time, at most one per time.
    pub events: Vec<Event>,
}

/// One timed event: at a time into the clip, a list of properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Milliseconds into the clip. The file may hold any value; the layer
    /// update skips one below 0 or more than 40 past the clip's end.
    pub time: i32,
    /// The properties, as key and value, in order.
    pub properties: Vec<(String, String)>,
}

impl Track {
    /// Builds a motion's list from the file's events, in file order:
    ///
    /// - Each event's properties are read into a new list (0x59431a,
    ///   0x594390). A key is looked up as the file spells it; if the list
    ///   holds it, its value is replaced, else the pair is added. Both are
    ///   stored in lowercase (0x63822a, the C locale's), so two keys that
    ///   differ only in case are both kept, in lowercase.
    /// - The list is inserted by time (0x5946fc). An event at a time the
    ///   list already holds replaces that event's whole list.
    ///
    /// The old prototype's `properties` and `ordered`
    /// (legacy/crates/sims-sim/src/pose/events.rs) do the same.
    pub fn build(bone: &str, events: &[sims_formats::animation::TimedProperty]) -> Self {
        let mut by_time = BTreeMap::new();
        for event in events {
            let mut properties: Vec<(String, String)> = Vec::new();
            for (key, value) in &event.pairs {
                match properties.iter_mut().find(|(k, _)| k == key) {
                    Some((_, old)) => *old = value.to_ascii_lowercase(),
                    None => properties.push((key.to_ascii_lowercase(), value.to_ascii_lowercase())),
                }
            }
            by_time.insert(event.time_ms, properties);
        }
        Self {
            bone: bone.to_owned(),
            events: by_time
                .into_iter()
                .map(|(time, properties)| Event { time, properties })
                .collect(),
        }
    }
}

/// Every clip of the animation files, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Animations {
    /// Each name's definitions, one per file that defines it, by the file's
    /// location relative to the install.
    clips: BTreeMap<String, BTreeMap<String, Clip>>,
    /// The sample files (`.cfp`), by name in lowercase, without the
    /// extension or any directory an archive member's name holds: each
    /// distinct content a file of that name has.
    samples: BTreeMap<String, BTreeSet<Vec<u8>>>,
}

impl Animations {
    /// Reads every animation file (`.bcf` members whose skeleton and
    /// appearance counts are 0) of the archives in `GameData/Animation`
    /// and of the indexed object directories, and notes every sample file
    /// (`.cfp`) there. A file that does not decode as an animation file is
    /// skipped, as [`crate::People`] skips skeleton files.
    ///
    /// That these are the files the original's library holds, and that it
    /// holds every clip in them, is **unverified**; the library's loader is
    /// not ported.
    pub fn load(install: &Install, index: &Index) -> Result<Self> {
        let mut resources = Vec::new();
        if let Some(directory) = resolve(&install.root, "GameData/Animation")? {
            for path in sorted_entries(&directory)? {
                let is_archive = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("far"));
                if is_archive && path.is_file() {
                    for member in all_far_members(&path)? {
                        resources.push(Resource {
                            mount: 0,
                            path: path.clone(),
                            member: Some(member),
                        });
                    }
                }
            }
        }
        resources.extend(index.resources.iter().cloned());
        let mut animations = Self::default();
        for resource in resources {
            let name = resource.name().to_ascii_lowercase();
            if let Some(stem) = name.strip_suffix(".cfp") {
                // An archive member's name can hold a directory, as
                // ExpansionShared.far's "Animations\xskill-....cfp" does;
                // the sample file is known by its own name.
                let file = stem.rsplit(['\\', '/']).next().unwrap_or(stem);
                let bytes = resource.read()?;
                animations
                    .samples
                    .entry(file.to_owned())
                    .or_default()
                    .insert(bytes);
                continue;
            }
            if !name.ends_with(".bcf") {
                continue;
            }
            let bytes = resource.read()?;
            if bytes.get(..8).is_none_or(|head| head != [0; 8]) {
                continue;
            }
            let Ok(parsed) = read_bcf(&bytes) else {
                continue;
            };
            let location = resource.location(&install.root);
            for animation in parsed {
                let clip = Clip {
                    skill: animation.skill,
                    duration: animation.duration_ms.to_bits(),
                    distance: animation.distance.to_bits(),
                    moving: animation.moving,
                    frames: animation.motions.first().map_or(0, |m| m.frames),
                    motions: animation.motions.len(),
                    tracks: animation
                        .motions
                        .iter()
                        .flat_map(|m| m.event_lists.iter().map(|list| (&m.bone, list)))
                        .filter(|(_, list)| !list.is_empty())
                        .map(|(bone, list)| Track::build(bone, list))
                        .collect(),
                    channels: animation
                        .motions
                        .iter()
                        .map(|m| Channel {
                            bone: m.bone.clone(),
                            translation: m.translation,
                            rotation: m.rotation,
                        })
                        .collect(),
                    samples: [animation.translation_count, animation.rotation_count],
                    name: animation.name,
                };
                animations
                    .clips
                    .entry(clip.name.clone())
                    .or_default()
                    .insert(location.clone(), clip);
            }
        }
        Ok(animations)
    }

    /// Adds a clip, defined by `file`. For tests.
    pub fn insert(&mut self, file: &str, clip: Clip) {
        self.clips
            .entry(clip.name.clone())
            .or_default()
            .insert(file.to_owned(), clip);
    }

    /// Notes a sample file, by its name without the extension. For tests.
    pub fn insert_samples(&mut self, skill: &str) {
        self.samples.entry(skill.to_ascii_lowercase()).or_default();
    }

    /// Adds a sample file's content, by its name without the extension.
    /// For tests.
    pub fn insert_sample_file(&mut self, skill: &str, bytes: Vec<u8>) {
        self.samples
            .entry(skill.to_ascii_lowercase())
            .or_default()
            .insert(bytes);
    }

    /// The positions and rotations of `clip`, from its sample file.
    ///
    /// Fails when no sample file has the clip's skill as its name, when
    /// two such files differ (which one the original would read is not
    /// known), and when the file does not decode.
    pub fn samples(&self, clip: &Clip) -> std::result::Result<Samples, Error> {
        let unsupported = |message: String| Err(Error::Unsupported(message));
        let mut files = self
            .samples
            .get(&clip.skill.to_ascii_lowercase())
            .into_iter()
            .flatten();
        let (Some(bytes), None) = (files.next(), files.next()) else {
            return unsupported(format!(
                "there is not exactly one sample file for the skill {:?}",
                clip.skill
            ));
        };
        read_cfp(bytes, clip.samples[0], clip.samples[1]).or_else(|e| {
            unsupported(format!(
                "the sample file of the skill {:?} does not decode: {e}",
                clip.skill
            ))
        })
    }

    /// Whether no clip is known.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// The clip named `name`, or why it cannot be told.
    ///
    /// A name no file spells exactly is looked up without regard to ASCII
    /// case. In the trace of the original's ticks, a script asks in tick
    /// 3 for "a2o-idle-neutral-trans-stand-Rstand" (as the global file's
    /// table spells it), the only file defines
    /// "a2o-idle-neutral-trans-stand-rstand", and the original loads that
    /// clip's samples during the tick and plays a clip of its duration
    /// and frame count. Where its code ignores the case is not found: the
    /// library's own table compares names byte for byte (0x5a27f2). So
    /// this rule is taken from what the original did, and a name that
    /// matches two names that differ only in case is refused.
    ///
    /// Also fails when no file defines the name; when two files define it
    /// differently (which one the library keeps depends on its load
    /// order, which is not ported); and when the clip's sample file is
    /// missing.
    pub fn clip(&self, name: &str) -> std::result::Result<&Clip, Error> {
        let unsupported = |message: String| Err(Error::Unsupported(message));
        let definitions = match self.clips.get(name) {
            Some(definitions) => definitions,
            None => {
                let mut others = self
                    .clips
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case(name));
                match (others.next(), others.next()) {
                    (Some((_, definitions)), None) => definitions,
                    (Some((first, _)), Some((second, _))) => {
                        return unsupported(format!(
                            "no animation file defines the clip {name:?}, but files define \
                             {first:?} and {second:?}, which differ from it only in case; \
                             which one the original would play is not known"
                        ));
                    }
                    _ => {
                        return unsupported(format!("no animation file defines the clip {name:?}"));
                    }
                }
            }
        };
        let mut clips = definitions.iter();
        let Some((file, clip)) = clips.next() else {
            return unsupported(format!("no animation file defines the clip {name:?}"));
        };
        if let Some((other, _)) = clips.find(|(_, other)| *other != clip) {
            return unsupported(format!(
                "the clip {name:?} is defined differently by {file} and by {other}; choosing \
                 between them needs the original's animation library, which is not ported"
            ));
        }
        if !self.samples.contains_key(&clip.skill.to_ascii_lowercase()) {
            return unsupported(format!(
                "the samples of the clip {name:?} ({}.cfp) are missing",
                clip.skill
            ));
        }
        Ok(clip)
    }
}
