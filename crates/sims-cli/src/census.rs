//! Corpus census: decode every resource under a directory with the matching
//! `sims-formats` reader and tally what fails and what has no reader yet.
//!
//! This answers "does the formats crate handle real content?" with data, for
//! the original install, expansion packs and custom content alike. It also
//! counts the primitives the decoded trees use, by pack, so the VM's
//! coverage can be weighed against what the content needs.

use sims_formats::{
    animation, avatar, behavior, career, execution, far, ffn, iff, lot, neighborhood, sprite,
    strings, tatt, tuning,
};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Default)]
struct Tally {
    total: u64,
    failed: u64,
    /// Failures grouped by message with digits elided, each with one example.
    failures: BTreeMap<String, (u64, String)>,
}

/// How deep archives inside archives are opened. The game's own archives
/// hold none.
const MAX_NESTING: u32 = 4;
/// How many member bytes one file on disk may lead the census to decode,
/// as a multiple of its own length, plus [`WORK_FLOOR`]. Archive members
/// may overlap, so without this a small archive could name the same large
/// member many times over.
const WORK_FACTOR: u64 = 4;
const WORK_FLOOR: u64 = 16 * 1024 * 1024;

#[derive(Default)]
pub struct Census {
    files: u64,
    /// Member bytes the current file on disk may still lead to.
    budget: u64,
    tallies: BTreeMap<String, Tally>,
    /// IFF chunk tags with no reader, with counts.
    unread_tags: BTreeMap<String, u64>,
    /// Other file extensions seen but not decoded, with counts.
    unread_files: BTreeMap<String, u64>,
    /// Decoded trees (BHAV chunks) per pack.
    trees: BTreeMap<String, u64>,
    /// Primitive instructions (opcodes below 256; the rest call trees)
    /// by opcode, then pack.
    primitives: BTreeMap<u16, BTreeMap<String, u64>>,
}

/// The pack a resource belongs to, from its path: the first component
/// naming an expansion pack (`ExpansionPack`, `ExpansionPack2`, ...) or
/// user data (`UserData`, `UserData2`, ...), else "Base".
fn pack(location: &str) -> String {
    Path::new(location)
        .components()
        .find_map(|c| {
            let name = c.as_os_str().to_string_lossy();
            if name.starts_with("ExpansionPack") {
                Some(name.into_owned())
            } else if name.starts_with("UserData") {
                Some("UserData".to_owned())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "Base".to_owned())
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

/// Where an archive's bytes are: in memory, as a member of another
/// archive, or in a file on disk of the given length.
enum Archive<'a> {
    Memory(&'a [u8]),
    Disk(&'a mut File, u64),
}

impl Archive<'_> {
    fn len(&self) -> u64 {
        match self {
            Self::Memory(bytes) => bytes.len() as u64,
            Self::Disk(_, len) => *len,
        }
    }

    /// `len` bytes from `offset`, which the caller has checked lie inside
    /// the archive. A read from disk is capped at [`far::MAX_MEMBER`].
    fn read(&mut self, offset: u64, len: u64) -> sims_formats::Result<Cow<'_, [u8]>> {
        let fail = |message: String| sims_formats::Error::new(offset as usize, message);
        match self {
            Self::Memory(bytes) => usize::try_from(offset)
                .ok()
                .zip(usize::try_from(len).ok())
                .and_then(|(start, len)| bytes.get(start..start.checked_add(len)?))
                .map(Cow::Borrowed)
                .ok_or_else(|| fail("range outside archive".to_owned())),
            Self::Disk(file, _) => {
                if len > far::MAX_MEMBER {
                    return Err(fail(format!("{len} bytes exceed the 128 MiB limit")));
                }
                let mut bytes = Vec::with_capacity(len as usize);
                file.seek(SeekFrom::Start(offset))
                    .and_then(|_| (&mut **file).take(len).read_to_end(&mut bytes))
                    .map_err(|e| fail(e.to_string()))?;
                if bytes.len() as u64 != len {
                    return Err(fail("archive ends early".to_owned()));
                }
                Ok(Cow::Owned(bytes))
            }
        }
    }
}

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
            self.disk_file(path)?;
        }
        Ok(())
    }

    /// Visits a file on disk. An archive is read a member at a time, so
    /// only its directory and one member are in memory at once; any other
    /// file is read whole, up to [`far::MAX_MEMBER`].
    fn disk_file(&mut self, path: &Path) -> std::io::Result<()> {
        let location = path.display().to_string();
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        self.budget = len.saturating_mul(WORK_FACTOR).saturating_add(WORK_FLOOR);
        let mut magic = [0; 8];
        if len >= 8 {
            file.read_exact(&mut magic)?;
        }
        if magic == *b"FAR!byAZ" {
            self.files += 1;
            self.far(&location, Archive::Disk(&mut file, len), 0);
        } else if len > far::MAX_MEMBER {
            self.files += 1;
            let error = sims_formats::Error::new(0, "file exceeds the 128 MiB limit");
            self.record("Oversized file", &location, Err(error));
        } else {
            file.rewind()?;
            let mut bytes = Vec::with_capacity(len as usize);
            file.take(len).read_to_end(&mut bytes)?;
            self.file(&location, &bytes, 0);
        }
        Ok(())
    }

    /// Visits one file or archive member. `depth` counts the archives it
    /// is inside.
    fn file(&mut self, location: &str, bytes: &[u8], depth: u32) {
        self.files += 1;
        if bytes.starts_with(b"FAR!byAZ") {
            self.far(location, Archive::Memory(bytes), depth);
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

    fn far(&mut self, location: &str, mut archive: Archive<'_>, depth: u32) {
        let entries = match self.members(&mut archive, depth) {
            Ok(entries) => {
                self.record("FAR archive", location, Ok(()));
                entries
            }
            Err(error) => return self.record("FAR archive", location, Err(error)),
        };
        for entry in entries {
            let location = format!("{location}/{}", entry.name);
            match archive.read(u64::from(entry.offset), u64::from(entry.stored_size)) {
                Ok(member) => self.file(&location, &member, depth + 1),
                Err(error) => {
                    self.files += 1;
                    self.record("FAR member", &location, Err(error));
                }
            }
        }
    }

    /// An archive's members, each lying between the header and the
    /// directory, so a member is never the archive itself and every
    /// archive inside one is smaller. Fails when the archive is nested too
    /// deeply or its members would exceed the work budget.
    fn members(
        &mut self,
        archive: &mut Archive<'_>,
        depth: u32,
    ) -> sims_formats::Result<Vec<far::Entry>> {
        if depth >= MAX_NESTING {
            return Err(sims_formats::Error::new(
                0,
                format!("FAR archive nested more than {MAX_NESTING} deep"),
            ));
        }
        let len = archive.len();
        let offset = far::manifest_offset(&archive.read(0, len.min(16))?, len)?;
        let directory_len = far::directory_len(offset, len)?;
        let directory = archive.read(u64::from(offset), directory_len as u64)?;
        let entries = far::directory(&directory, len)?;
        if let Some(entry) = entries.iter().find(|e| !e.within_payload(offset)) {
            return Err(sims_formats::Error::new(
                entry.offset as usize,
                format!(
                    "FAR member {} overlaps the archive header or directory",
                    entry.name
                ),
            ));
        }
        let work = entries
            .iter()
            .map(|e| u64::from(e.stored_size))
            .fold(0u64, u64::saturating_add);
        if work > self.budget {
            return Err(sims_formats::Error::new(
                offset as usize,
                "FAR members exceed the census work budget",
            ));
        }
        self.budget -= work;
        Ok(entries)
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
            if &chunk.tag == b"BHAV" {
                let outcome = behavior::Behavior::parse(file.data(chunk));
                if let Ok(tree) = &outcome {
                    self.count_primitives(location, tree);
                }
                self.record("BHAV chunk", &at, outcome.map(drop));
                continue;
            }
            match decode(&file, chunk) {
                Some(outcome) => self.record(&format!("{name} chunk"), &at, outcome),
                None => *self.unread_tags.entry(name).or_default() += 1,
            }
        }
    }

    fn count_primitives(&mut self, location: &str, tree: &behavior::Behavior) {
        let pack = pack(location);
        *self.trees.entry(pack.clone()).or_default() += 1;
        for instruction in tree.instructions.iter().filter(|i| i.opcode < 256) {
            *self
                .primitives
                .entry(instruction.opcode)
                .or_default()
                .entry(pack.clone())
                .or_default() += 1;
        }
    }

    /// The primitives table: one row per opcode used, one column per pack.
    fn primitives_report(&self, out: &mut String) {
        let packs: Vec<&String> = self.trees.keys().collect();
        let _ = writeln!(out, "\nPrimitives used by decoded trees, by pack:");
        let _ = write!(out, "  {:<4} {:<36} {:>9}", "op", "name", "total");
        for pack in &packs {
            let _ = write!(out, " {:>15}", pack);
        }
        let _ = write!(
            out,
            "\n  {:<4} {:<36} {:>9}",
            "",
            "trees",
            self.trees.values().sum::<u64>()
        );
        for pack in &packs {
            let _ = write!(out, " {:>15}", self.trees[*pack]);
        }
        let _ = writeln!(out);
        for (opcode, by_pack) in &self.primitives {
            let name = sims_vm::primitive_name(*opcode);
            let total: u64 = by_pack.values().sum();
            let _ = write!(out, "  {opcode:<4} {name:<36} {total:>9}");
            for pack in &packs {
                let _ = write!(out, " {:>15}", by_pack.get(*pack).copied().unwrap_or(0));
            }
            let _ = writeln!(out);
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
        self.primitives_report(&mut out);
        out
    }
}

/// Decodes one chunk fully (including every sprite frame), or returns None
/// when no reader exists for its type.
fn decode(file: &iff::Iff, chunk: &iff::Chunk) -> Option<Outcome> {
    let data = file.data(chunk);
    let outcome = match &chunk.tag {
        // BHAV chunks are decoded by `Census::iff`, which also counts
        // their primitives.
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
            // Sets that name no palette get theirs from the drawing code;
            // a placeholder still checks the encoding.
            let placeholder = sprite::Palette(vec![[255; 3]; 256]);
            (0..set.frames.len()).try_for_each(|i| match set.palette_id() {
                Some(_) => set.decode(data, i, file).map(drop),
                None => set.decode_with_palette(data, i, &placeholder).map(drop),
            })
        }),
        b"FAMI" => neighborhood::Family::parse(data).map(drop),
        b"NBRS" => neighborhood::Neighbors::parse(data).map(drop),
        b"NGBH" => neighborhood::Neighborhood::parse(data).map(drop),
        b"Arry" => lot::Array::parse(data).map(drop),
        b"objt" => lot::ObjectTypes::parse(data).map(drop),
        b"SIMI" => lot::Simulator::parse(data).map(drop),
        // HOUS #0 is the lot's view settings; the house model is #1.
        b"HOUS" if chunk.id == 0 => lot::HouseView::parse(data).map(drop),
        b"HOUS" => lot::House::parse(data).map(drop),
        b"FLRm" => lot::surface_map(data, chunk.id, true).map(drop),
        b"WALm" => lot::surface_map(data, chunk.id, false).map(drop),
        b"rsmp" => iff::ResourceMap::parse(data).map(drop),
        _ => return None,
    };
    Some(outcome)
}

#[cfg(test)]
mod tests {
    use super::Census;

    /// A FAR 1a archive with the given directory entries, as (offset,
    /// size, name), after `payload`.
    fn archive(payload: &[u8], entries: &[(u32, u32, &str)]) -> Vec<u8> {
        let mut bytes = b"FAR!byAZ".to_vec();
        bytes.extend(1u32.to_le_bytes());
        bytes.extend((16 + payload.len() as u32).to_le_bytes());
        bytes.extend(payload);
        bytes.extend((entries.len() as u32).to_le_bytes());
        for (offset, size, name) in entries {
            for word in [*size, *size, *offset, name.len() as u32] {
                bytes.extend(word.to_le_bytes());
            }
            bytes.extend(name.as_bytes());
        }
        bytes
    }

    fn census(name: &str, bytes: &[u8]) -> String {
        let path =
            std::env::temp_dir().join(format!("sims-census-{name}-{}.far", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let mut census = Census::default();
        census.add_path(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        census.report()
    }

    /// An IFF file holding one BHAV per entry, each with the given
    /// instruction opcodes.
    fn trees(trees: &[&[u16]]) -> Vec<u8> {
        let mut bytes = vec![0; 64];
        bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
        for (id, opcodes) in trees.iter().enumerate() {
            let mut data = 0x8002u16.to_le_bytes().to_vec();
            data.extend((opcodes.len() as u16).to_le_bytes());
            data.extend([0; 8]);
            for &opcode in *opcodes {
                data.extend(opcode.to_le_bytes());
                data.extend([254, 255]);
                data.extend([0; 8]);
            }
            bytes.extend(b"BHAV");
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend((id as u16).to_be_bytes());
            bytes.extend([0; 66]);
            bytes.extend(data);
        }
        bytes
    }

    #[test]
    fn primitives_are_counted_by_pack_and_tree_calls_are_not() {
        let dir = std::env::temp_dir().join(format!("sims-census-prims-{}", std::process::id()));
        let expansion = dir.join("ExpansionPack3");
        std::fs::create_dir_all(&expansion).unwrap();
        std::fs::write(dir.join("base.iff"), trees(&[&[2, 2, 0x100], &[44]])).unwrap();
        std::fs::write(expansion.join("hot.iff"), trees(&[&[2, 0x1000]])).unwrap();
        let mut census = Census::default();
        census.add_path(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let report = census.report();
        let table = &report[report.find("Primitives used").expect("table")..];
        let row = |op: &str| {
            table
                .lines()
                .find(|l| l.split_whitespace().next() == Some(op))
                .unwrap_or_else(|| panic!("no row {op} in {table}"))
                .split_whitespace()
                .collect::<Vec<_>>()
        };
        assert_eq!(row("trees")[1..], ["3", "2", "1"], "{table}");
        let expression = row("2");
        assert_eq!(
            expression[expression.len() - 3..],
            ["3", "2", "1"],
            "{table}"
        );
        let animate = row("44");
        assert_eq!(animate[animate.len() - 3..], ["1", "1", "0"], "{table}");
        assert!(
            !table.lines().any(|l| l.trim_start().starts_with("256 ")),
            "{table}"
        );
        assert!(report.contains("BHAV chunk"), "{report}");
    }

    #[test]
    fn an_archive_member_spanning_the_archive_is_rejected() {
        // Its only member is the whole 37-byte archive: opening it again
        // would recurse without end.
        let mut bytes = b"FAR!byAZ".to_vec();
        for n in [1u32, 16, 1, 37, 37, 0, 1] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.push(b'x');
        let report = census("self", &bytes);
        assert!(report.contains("overlaps the archive header"), "{report}");
        assert!(report.contains("visited: 1\n"), "{report}");
    }

    #[test]
    fn an_archive_on_disk_claiming_a_huge_directory_is_not_read() {
        let path =
            std::env::temp_dir().join(format!("sims-census-huge-{}.far", std::process::id()));
        // The directory starts right after the header, so the whole sparse
        // remainder would be the directory.
        let mut bytes = b"FAR!byAZ".to_vec();
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(16u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(sims_formats::far::MAX_DIRECTORY + 17)
            .unwrap();
        let mut census = Census::default();
        census.add_path(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let report = census.report();
        assert!(report.contains("64 MiB"), "{report}");
    }

    #[test]
    fn archives_inside_archives_stop_at_the_nesting_limit() {
        let mut bytes = archive(b"inner", &[(16, 5, "x.bin")]);
        for _ in 0..5 {
            let len = bytes.len() as u32;
            bytes = archive(&bytes, &[(16, len, "n.far")]);
        }
        let report = census("nested", &bytes);
        assert!(report.contains("nested more than 4 deep"), "{report}");
        // The outer archive and the four inside it it opens.
        assert!(report.contains("visited: 5\n"), "{report}");
    }

    #[test]
    fn overlapping_members_cannot_exceed_the_work_budget() {
        // Every entry names the same large member.
        let payload = vec![0; 1 << 20];
        let entries: Vec<_> = (0..100).map(|_| (16, 1 << 20, "same.bin")).collect();
        let report = census("overlap", &archive(&payload, &entries));
        assert!(report.contains("work budget"), "{report}");
        let honest = census("honest", &archive(&payload, &entries[..1]));
        assert!(honest.contains("visited: 2\n"), "{honest}");
    }
}
