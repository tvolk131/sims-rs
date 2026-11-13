//! Corpus census: decode every resource under a directory with the matching
//! `sims-formats` reader and tally what fails and what has no reader yet.
//!
//! This answers "does the formats crate handle real content?" with data, for
//! the original install, expansion packs and custom content alike.

use sims_formats::{
    animation, avatar, behavior, career, execution, far, ffn, iff, lot, neighborhood, sprite,
    strings, tatt, tuning,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

#[derive(Default)]
struct Tally {
    total: u64,
    failed: u64,
    /// Failures grouped by message with digits elided, each with one example.
    failures: BTreeMap<String, (u64, String)>,
}

#[derive(Default)]
pub struct Census {
    files: u64,
    tallies: BTreeMap<String, Tally>,
    /// IFF chunk tags with no reader, with counts.
    unread_tags: BTreeMap<String, u64>,
    /// Other file extensions seen but not decoded, with counts.
    unread_files: BTreeMap<String, u64>,
}

fn pattern(message: &str) -> String {
    let mut out = String::new();
    let mut in_number = false;
    for c in message.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                out.push('#');
            }
            in_number = true;
        } else {
            in_number = false;
            out.push(c);
        }
    }
    out
}

fn tag_name(tag: &[u8; 4]) -> String {
    tag.iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

type Outcome = sims_formats::Result<()>;

impl Census {
    fn record(&mut self, kind: &str, location: &str, outcome: Outcome) {
        let tally = self.tallies.entry(kind.to_owned()).or_default();
        tally.total += 1;
        if let Err(error) = outcome {
            tally.failed += 1;
            let entry = tally
                .failures
                .entry(pattern(&error.message))
                .or_insert_with(|| (0, format!("{location}: {error}")));
            entry.0 += 1;
        }
    }

    /// Walks a file or directory in sorted order, so reports are stable.
    pub fn add_path(&mut self, path: &Path) -> std::io::Result<()> {
        if path.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<_, _>>()?;
            entries.sort();
            for entry in entries {
                self.add_path(&entry)?;
            }
        } else if path.is_file() {
            let bytes = std::fs::read(path)?;
            self.file(&path.display().to_string(), &bytes);
        }
        Ok(())
    }

    fn file(&mut self, location: &str, bytes: &[u8]) {
        self.files += 1;
        if bytes.starts_with(b"FAR!byAZ") {
            self.far(location, bytes);
        } else if bytes.starts_with(b"IFF FILE") {
            self.iff(location, bytes);
        } else {
            let ext = extension(location);
            let outcome: Option<Outcome> = match ext.as_str() {
                "bmf" => Some(avatar::read_bmf(bytes).map(drop)),
                "cmx" => Some(avatar::read_cmx(bytes).map(drop)),
                "skn" => Some(avatar::read_skn(bytes).map(drop)),
                "ffn" => Some(ffn::Font::parse(bytes).map(drop)),
                "bcf" => {
                    // A BCF holds skeletons and appearances, or animations.
                    // The two readers each accept one kind.
                    let outcome = avatar::read_bcf(bytes)
                        .map(drop)
                        .or_else(|_| animation::read_bcf(bytes).map(drop));
                    Some(outcome)
                }
                _ => None,
            };
            match outcome {
                Some(outcome) => self.record(&format!(".{ext} file"), location, outcome),
                None => *self.unread_files.entry(format!(".{ext}")).or_default() += 1,
            }
        }
    }

    fn far(&mut self, location: &str, bytes: &[u8]) {
        let len = bytes.len() as u64;
        let directory = far::manifest_offset(&bytes[..bytes.len().min(16)], len)
            .and_then(|offset| far::directory(&bytes[offset as usize..], len));
        let entries = match directory {
            Ok(entries) => {
                self.record("FAR archive", location, Ok(()));
                entries
            }
            Err(error) => return self.record("FAR archive", location, Err(error)),
        };
        for entry in entries {
            let start = entry.offset as usize;
            let member = &bytes[start..start + entry.stored_size as usize];
            self.file(&format!("{location}/{}", entry.name), member);
        }
    }

    fn iff(&mut self, location: &str, bytes: &[u8]) {
        let file = match iff::Iff::parse(bytes.to_vec()) {
            Ok(file) => {
                self.record("IFF file", location, Ok(()));
                file
            }
            Err(error) => return self.record("IFF file", location, Err(error)),
        };
        let mut tags: Vec<[u8; 4]> = file.chunks.iter().map(|c| c.tag).collect();
        tags.sort();
        tags.dedup();
        for tag in &tags {
            let outcome = file.resource_order(tag).map(drop);
            self.record("IFF resource order", location, outcome);
        }
        for chunk in &file.chunks {
            let name = tag_name(&chunk.tag);
            let at = format!("{location} {name} #{} {:?}", chunk.id, chunk.label);
            match decode(&file, chunk) {
                Some(outcome) => self.record(&format!("{name} chunk"), &at, outcome),
                None => *self.unread_tags.entry(name).or_default() += 1,
            }
        }
    }

    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Files and archive members visited: {}\n", self.files);
        let _ = writeln!(out, "{:<24} {:>9} {:>9}", "Decoded", "total", "failed");
        for (kind, tally) in &self.tallies {
            let _ = writeln!(out, "{kind:<24} {:>9} {:>9}", tally.total, tally.failed);
        }
        let _ = writeln!(out, "\nFailures by message (count, then one example):");
        for (kind, tally) in &self.tallies {
            let mut failures: Vec<_> = tally.failures.iter().collect();
            failures.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
            for (message, (count, example)) in failures {
                let _ = writeln!(out, "  [{kind}] {count} x {message}\n      e.g. {example}");
            }
        }
        let _ = writeln!(out, "\nIFF chunk types with no reader:");
        let mut unread: Vec<_> = self.unread_tags.iter().collect();
        unread.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (tag, count) in unread {
            let _ = writeln!(out, "  {tag:<6} {count}");
        }
        let _ = writeln!(out, "\nFile types with no reader:");
        let mut unread: Vec<_> = self.unread_files.iter().collect();
        unread.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (ext, count) in unread {
            let _ = writeln!(out, "  {ext:<8} {count}");
        }
        out
    }
}

/// Decodes one chunk fully (including every sprite frame), or returns None
/// when no reader exists for its type.
fn decode(file: &iff::Iff, chunk: &iff::Chunk) -> Option<Outcome> {
    let data = file.data(chunk);
    let outcome = match &chunk.tag {
        b"BHAV" => behavior::Behavior::parse(data).map(drop),
        b"OBJD" => behavior::ObjectDefinition::parse(data).map(drop),
        b"TTAB" => behavior::InteractionTable::parse(data).map(drop),
        b"STR#" | b"CTSS" | b"TTAs" | b"FAMs" => strings::StringTable::parse(data).map(drop),
        b"GLOB" => execution::global_link(data).map(drop),
        b"BCON" => execution::constants(data).map(drop),
        b"OBJf" => execution::ObjectFunctions::parse(data).map(drop),
        b"SLOT" => execution::Slots::parse(data).map(drop),
        b"FCNS" => tuning::Constants::parse(data).map(drop),
        b"TATT" => tatt::Tatt::parse(data).map(drop),
        b"CARR" => career::Career::parse(data).map(drop),
        b"PALT" => sprite::Palette::parse(data).map(drop),
        b"DGRP" => sprite::DrawGroup::parse(data).map(drop),
        b"SPR2" => sprite::SpriteSet::parse(data).and_then(|set| {
            (0..set.frames.len()).try_for_each(|i| set.decode(data, i, file).map(drop))
        }),
        b"SPR#" => sprite::LegacySpriteSet::parse(data).and_then(|set| {
            (0..set.frames.len()).try_for_each(|i| set.decode(data, i, file).map(drop))
        }),
        b"FAMI" => neighborhood::Family::parse(data).map(drop),
        b"NBRS" => neighborhood::Neighbors::parse(data).map(drop),
        b"NGBH" => neighborhood::Neighborhood::parse(data).map(drop),
        b"Arry" => lot::Array::parse(data).map(drop),
        b"objt" => lot::ObjectTypes::parse(data).map(drop),
        b"SIMI" => lot::Simulator::parse(data).map(drop),
        b"HOUS" => lot::House::parse(data).map(drop),
        b"FLRm" => lot::surface_map(data, chunk.id, true).map(drop),
        b"WALm" => lot::surface_map(data, chunk.id, false).map(drop),
        b"rsmp" => iff::ResourceMap::parse(data).map(drop),
        _ => return None,
    };
    Some(outcome)
}
