//! The animation clips a person's animator can play, and what of each one
//! the simulation reads.
//!
//! The original looks a clip up by name in its animation library
//! (0x59dc32, through 0x5a03e4) and builds a layer from it (0x597a88). Of
//! the clip the layer keeps its duration (clip +0x2c), the frame count of
//! its first motion (layer +0x38), and one binding per motion (layer +8),
//! each with a cursor over the motion's timed events. The samples (the
//! `.cfp` file named after the clip's skill) are only read to pose the
//! skeleton, which is presentation; a clip whose samples are missing is
//! still refused, since how the original copes with that is not ported.

use crate::index::{far_members, sorted_entries};
use crate::install::resolve;
use crate::{Error, Index, Install, Resource, Result};
use sims_formats::animation::read_bcf;
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
    /// The frame count of the clip's first motion, or 0 for a clip
    /// without motions (layer +0x38, 0x597b01).
    pub frames: u32,
    /// How many motions the clip has.
    pub motions: usize,
    /// Whether any motion has a timed event. The layer update dispatches
    /// those (0x5912fd), which is not ported.
    pub events: bool,
}

/// Every clip of the animation files, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Animations {
    /// Each name's definitions, one per file that defines it, by the file's
    /// location relative to the install.
    clips: BTreeMap<String, BTreeMap<String, Clip>>,
    /// The names of the sample files (`.cfp`), in lowercase, without the
    /// extension.
    samples: BTreeSet<String>,
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
                    for member in far_members(&path)? {
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
                animations.samples.insert(stem.to_owned());
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
                    frames: animation.motions.first().map_or(0, |m| m.frames),
                    motions: animation.motions.len(),
                    events: animation
                        .motions
                        .iter()
                        .any(|m| m.event_lists.iter().any(|list| !list.is_empty())),
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
        self.samples.insert(skill.to_ascii_lowercase());
    }

    /// Whether no clip is known.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// The clip named `name`, or why it cannot be told.
    ///
    /// Fails when no file defines the name; when only a name that differs
    /// in case does (whether the original's library ignores case is not
    /// known); when two files define it differently (which one the library
    /// keeps depends on its load order, which is not ported); and when the
    /// clip's sample file is missing.
    pub fn clip(&self, name: &str) -> std::result::Result<&Clip, Error> {
        let unsupported = |message: String| Err(Error::Unsupported(message));
        let Some(definitions) = self.clips.get(name) else {
            if let Some(other) = self.clips.keys().find(|k| k.eq_ignore_ascii_case(name)) {
                return unsupported(format!(
                    "no animation file defines the clip {name:?}, but one defines {other:?}; \
                     whether the original's animation library ignores case is not known"
                ));
            }
            return unsupported(format!("no animation file defines the clip {name:?}"));
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
        if !self.samples.contains(&clip.skill.to_ascii_lowercase()) {
            return unsupported(format!(
                "the samples of the clip {name:?} ({}.cfp) are missing",
                clip.skill
            ));
        }
        Ok(clip)
    }
}
