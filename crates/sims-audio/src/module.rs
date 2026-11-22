//! Sound modules: a HIT program with its HSM symbols and HOT tables.
//!
//! The three files of a module share a directory and a file stem, which
//! may differ in case (`SoundData\simssound.HIT` goes with
//! `SoundData\SimsSound.hot`). The install holds 27 modules: the base
//! game's two in `SoundData`, one per expansion pack, and one per
//! downloaded object that brings its own sounds.

use crate::files::{Entry, SoundFiles};
use crate::hit::Program;
use crate::hot::{Hot, Value};
use crate::hsm::Symbols;
use crate::{Error, Result};
use std::collections::BTreeMap;

/// One module, parsed.
#[derive(Debug, Clone)]
pub struct Module {
    /// The path of the HIT file, without its extension, lowercased.
    pub name: String,
    pub program: Program,
    pub symbols: Symbols,
    pub hot: Hot,
}

impl Module {
    /// Parses one module from its three files' bytes.
    pub fn parse(name: &str, hit: &[u8], hsm: &[u8], hot: &[u8]) -> Result<Self> {
        Ok(Self {
            name: name.to_owned(),
            program: Program::parse(hit)?,
            symbols: Symbols::parse(hsm)?,
            hot: Hot::parse(hot)?,
        })
    }

    /// Whether a number is one of this program's track data IDs.
    pub fn is_target(&self, id: u32) -> bool {
        self.hot.track_data.contains_key(&id)
    }

    /// The address a track data ID stands for.
    pub fn address(&self, id: u32) -> Option<u32> {
        self.hot.track_data.get(&id).copied()
    }

    /// The number a HOT value names, with this module's equates and
    /// symbols.
    pub fn number(&self, value: &Value) -> Option<i64> {
        self.hot.number(value, &self.symbols)
    }

    /// The track data ID of a track's entry point.
    pub fn entry(&self, track: u32) -> Option<u32> {
        let track = self.hot.tracks.get(&track)?;
        self.number(&track.entry)
            .and_then(|n| u32::try_from(n).ok())
    }

    /// Loads every module of an install's sound files, sorted by name. A
    /// HIT file without its HSM or HOT file is an error.
    pub fn load_all(files: &SoundFiles) -> Result<Vec<Self>> {
        let mut parts: BTreeMap<String, [Option<&Entry>; 3]> = BTreeMap::new();
        for entry in files.iter() {
            let extension = entry.extension();
            let slot = match extension.as_str() {
                "hit" => 0,
                "hsm" => 1,
                "hot" => 2,
                _ => continue,
            };
            let key = crate::files::key(&entry.path);
            let stem = &key[..key.len() - extension.len() - 1];
            parts.entry(stem.to_owned()).or_default()[slot] = Some(entry);
        }
        let list: Vec<(String, [Option<&Entry>; 3])> = parts.into_iter().collect();
        let load_one = |(name, [hit, hsm, hot]): &(String, [Option<&Entry>; 3])| -> Result<Self> {
            let module_error = |message: String| Error::Module {
                name: name.clone(),
                message,
            };
            let missing = |what: &str| module_error(format!("no {what} file"));
            let hit = hit.ok_or_else(|| missing("HIT"))?.read()?;
            let hsm = hsm.ok_or_else(|| missing("HSM"))?.read()?;
            let hot = hot.ok_or_else(|| missing("HOT"))?.read()?;
            Self::parse(name, &hit, &hsm, &hot).map_err(|e| module_error(e.to_string()))
        };
        let workers = if list.len() <= 1 {
            1
        } else {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .clamp(1, 8)
                .min(list.len())
        };
        let loaded: Vec<Result<Self>> = if workers <= 1 {
            list.iter().map(load_one).collect()
        } else {
            let chunk_size = list.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = list
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || chunk.iter().map(load_one).collect::<Vec<_>>())
                    })
                    .collect();
                let mut out = Vec::with_capacity(list.len());
                for handle in handles {
                    out.extend(handle.join().expect("sound module worker panicked"));
                }
                out
            })
        };
        loaded.into_iter().collect()
    }
}
