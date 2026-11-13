//! Read-only, deterministic content indexing. Duplicate names retain separate sources.
pub mod assets;
pub mod autonomy_resources;
pub mod avatar;
pub mod avatar_catalog;
pub mod avatar_installation;
pub mod avatar_material;
pub mod avatar_mount;
mod body_profile;
pub mod careers;
pub mod catalog;
pub mod character_appearance;
pub mod character_censor;
pub mod character_geometry;
pub mod character_initialization;
pub mod character_loading;
pub mod character_model;
pub mod character_portrait;
pub mod clothing_catalog;
pub mod clothing_profile;
pub mod compatibility;
#[cfg(test)]
#[path = "resource_scan/descriptor_tests.rs"]
mod descriptor_scan_tests;
pub mod dialog;
pub mod domestic;
pub mod graphic_initialization;
pub mod graphic_loading;
pub mod graphic_materials;
pub mod graphic_profiles;
pub mod graphic_retirement;
pub mod graphic_update;
pub mod house_loading;
pub mod house_startup;
pub mod house_valuation;
pub mod household;
pub mod neighborhood_input;
pub mod notifications;
pub mod object_catalog;
pub mod object_graphics;
pub mod object_resources;
pub mod owned_resources;
mod resource_scan;
mod startup_inputs;
mod startup_memo;
pub mod save_storage;
pub mod saved_avatar;
pub mod saved_layers;
pub mod saves;
pub mod skeleton_resources;
pub mod stairs;
pub mod surfaces;
pub mod vm;
pub mod zoning;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sims_formats::{
    behavior::{Behavior, InteractionTable, ObjectDefinition},
    far,
    iff::Iff,
    sprite::{DrawGroup, Palette, SpriteSet},
    strings::StringTable,
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
const MAX_RESOURCE: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Resource {
    pub locator: String,
    pub name: String,
    pub source: PathBuf,
    pub offset: u64,
    pub size: u64,
    pub unpacked_size: u64,
    pub archived: bool,
    /// Managed-save bytes take precedence over physical files. Never opened
    /// through the original save path; immutable assets keep the file backend.
    #[serde(skip)]
    pub owned: Option<std::sync::Arc<[u8]>>,
}
impl Resource {
    fn validate_storage(&self) -> Result<()> {
        if self.size > MAX_RESOURCE {
            bail!("{} exceeds 128 MiB resource limit", self.locator);
        }
        if self.size != self.unpacked_size {
            bail!("compressed FAR entry is unsupported: {}", self.locator);
        }
        Ok(())
    }
    pub fn read(&self) -> Result<Vec<u8>> {
        self.validate_storage()?;
        if let Some(bytes) = &self.owned {
            anyhow::ensure!(
                bytes.len() as u64 == self.size,
                "owned resource size differs: {}",
                self.locator
            );
            return Ok(bytes.to_vec());
        }
        let mut f =
            File::open(&self.source).with_context(|| format!("open {}", self.source.display()))?;
        self.read_file(&mut f)
    }
    fn read_file(&self, f: &mut File) -> Result<Vec<u8>> {
        if self
            .offset
            .checked_add(self.size)
            .is_none_or(|end| end > f.metadata().map(|m| m.len()).unwrap_or(0))
        {
            bail!("resource outside source file: {}", self.locator);
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = vec![0; self.size as usize];
        f.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    pub fn is_iff(&self) -> bool {
        self.name.to_ascii_lowercase().ends_with(".iff")
    }
}
#[derive(Debug, Clone)]
pub struct Index {
    pub root: PathBuf,
    pub resources: Vec<Resource>,
    pub archives: usize,
    pub diagnostics: Vec<String>,
}
impl Index {
    pub fn scan(root: &Path) -> Result<Self> {
        if !root.is_dir() {
            bail!("content directory not found: {}", root.display());
        }
        let mut paths = Vec::new();
        walk(root, &mut paths)?;
        paths.sort();
        let mut index = Self {
            root: root.to_path_buf(),
            resources: Vec::new(),
            archives: 0,
            diagnostics: Vec::new(),
        };
        for p in paths {
            let relative = p.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
            let len = p.metadata()?.len();
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("far")) {
                match read_archive(&p, len) {
                    Ok(entries) => {
                        index.archives += 1;
                        for e in entries {
                            index.resources.push(Resource {
                                locator: format!("{relative}::{}", e.name),
                                name: e.name,
                                source: p.clone(),
                                offset: u64::from(e.offset),
                                size: u64::from(e.stored_size),
                                unpacked_size: u64::from(e.unpacked_size),
                                archived: true,
                                owned: None,
                            });
                        }
                    }
                    Err(e) => index.diagnostics.push(format!("{relative}: {e:#}")),
                }
            } else {
                index.resources.push(Resource {
                    locator: relative,
                    name: p.file_name().unwrap().to_string_lossy().into_owned(),
                    source: p,
                    offset: 0,
                    size: len,
                    unpacked_size: len,
                    archived: false,
                    owned: None,
                });
            }
        }
        index.resources.sort_by(|a, b| a.locator.cmp(&b.locator));
        Ok(index)
    }
    pub fn find(&self, selector: &str) -> Result<usize> {
        let exact: Vec<_> = self
            .resources
            .iter()
            .enumerate()
            .filter(|(_, r)| r.locator.eq_ignore_ascii_case(selector))
            .map(|(i, _)| i)
            .collect();
        match exact.as_slice() {
            [i] => return Ok(*i),
            [] => (),
            _ => bail!(
                "ambiguous case-insensitive locator {selector}; source contains duplicate entries"
            ),
        }
        let candidates: Vec<_> = self
            .resources
            .iter()
            .enumerate()
            .filter(|(_, r)| r.name.eq_ignore_ascii_case(selector))
            .collect();
        match candidates.as_slice() {
            [(i, _)] => Ok(*i),
            [] => bail!("resource not found: {selector}"),
            _ => bail!(
                "ambiguous resource {selector}; use its full locator:\n{}",
                candidates
                    .iter()
                    .map(|(_, r)| r.locator.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        }
    }
    pub fn document(&self, id: usize) -> Result<Document> {
        let res = self
            .resources
            .get(id)
            .context("invalid resource selection")?;
        let iff = Iff::parse(res.read()?).with_context(|| res.locator.clone())?;
        Ok(Document::new(res.locator.clone(), iff))
    }
}
fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            walk(&entry.path(), out)?;
        } else if ty.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}
fn read_archive(path: &Path, len: u64) -> Result<Vec<far::Entry>> {
    let mut f = File::open(path)?;
    let mut header = [0; 16];
    f.read_exact(&mut header)?;
    let offset = far::manifest_offset(&header, len)?;
    let directory_size = len - u64::from(offset);
    if directory_size > 64 * 1024 * 1024 {
        bail!("FAR directory exceeds 64 MiB limit");
    }
    f.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut directory = vec![0; directory_size as usize];
    f.read_exact(&mut directory)?;
    Ok(far::directory(&directory, len)?)
}
#[derive(Debug, Clone)]
pub struct Named<T> {
    pub id: u16,
    pub name: String,
    pub value: T,
}
#[derive(Debug, Clone)]
pub struct Document {
    pub source: String,
    pub iff: Iff,
    pub objects: Vec<Named<ObjectDefinition>>,
    pub groups: Vec<Named<DrawGroup>>,
    pub behaviors: Vec<Named<Behavior>>,
    pub tables: Vec<Named<InteractionTable>>,
    pub diagnostics: Vec<String>,
}
impl Document {
    pub fn new(source: String, iff: Iff) -> Self {
        let mut doc = Self {
            source,
            iff,
            objects: Vec::new(),
            groups: Vec::new(),
            behaviors: Vec::new(),
            tables: Vec::new(),
            diagnostics: Vec::new(),
        };
        for chunk in &doc.iff.chunks {
            let data = doc.iff.data(chunk);
            macro_rules! parse {
                ($ty:ty,$field:ident) => {
                    match <$ty>::parse(data) {
                        Ok(value) => doc.$field.push(Named {
                            id: chunk.id,
                            name: chunk.label.clone(),
                            value,
                        }),
                        Err(e) => doc.diagnostics.push(format!(
                            "{} #{} at {:#X}: {e}",
                            chunk.kind(),
                            chunk.id,
                            chunk.offset
                        )),
                    }
                };
            }
            match &chunk.tag {
                b"OBJD" => parse!(ObjectDefinition, objects),
                b"DGRP" => parse!(DrawGroup, groups),
                b"BHAV" => parse!(Behavior, behaviors),
                b"TTAB" => parse!(InteractionTable, tables),
                _ => (),
            }
        }
        doc.objects.sort_by_key(|o| o.id);
        doc.groups.sort_by_key(|o| o.id);
        doc.behaviors.sort_by_key(|o| o.id);
        doc.tables.sort_by_key(|o| o.id);
        doc
    }
    pub fn strings(&self, tag: &[u8; 4], id: u16) -> Result<StringTable> {
        let chunk = self.iff.get(tag, id).context("string table not present")?;
        Ok(StringTable::parse(self.iff.data(chunk))?)
    }
    pub fn object_title(&self, object: &Named<ObjectDefinition>) -> String {
        self.strings(b"CTSS", object.value.catalog)
            .ok()
            .and_then(|t| t.get(0, 1).map(str::to_owned))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| object.name.clone())
    }
    pub fn behavior_name(&self, id: u16) -> Option<String> {
        self.behaviors
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.name.clone())
    }
    pub fn menu_text(&self, table: &Named<InteractionTable>) -> String {
        let strings = self.strings(b"TTAs", table.id);
        let mut out = format!(
            "TTAB #{} · version {} · {}\n",
            table.id, table.value.version, table.name
        );
        for entry in &table.value.interactions {
            let label = strings
                .as_ref()
                .ok()
                .and_then(|t| t.get(entry.string_index as usize, 1))
                .unwrap_or("[missing menu string]");
            out.push_str(&format!("\n{label}\n  Action: {:#06x} {}\n  Test:   {:#06x} {}\n  Flags: {:#010x}   Autonomy threshold: {}\n",entry.action,self.behavior_name(entry.action).unwrap_or_else(||"[external/unresolved]".into()),entry.test,self.behavior_name(entry.test).unwrap_or_else(||"[external/unresolved]".into()),entry.flags,entry.autonomy));
        }
        if let Err(e) = strings {
            out.push_str(&format!("\nString table: {e:#}\n"));
        }
        out
    }
}
#[derive(Debug, Serialize, Default)]
pub struct Audit {
    pub resources: usize,
    pub archives: usize,
    pub iff_files: usize,
    pub chunks: BTreeMap<String, usize>,
    pub typed_ok: BTreeMap<String, usize>,
    pub failures: Vec<String>,
    pub decoded_frames: usize,
}
pub fn audit(index: &Index, decode_sprites: bool) -> Audit {
    let mut report = Audit {
        resources: index.resources.len(),
        archives: index.archives,
        failures: index.diagnostics.clone(),
        ..Audit::default()
    };
    for resource in index.resources.iter().filter(|r| r.is_iff()) {
        let iff = match resource.read().and_then(|b| Ok(Iff::parse(b)?)) {
            Ok(iff) => iff,
            Err(e) => {
                report.failures.push(format!("{}: {e:#}", resource.locator));
                continue;
            }
        };
        report.iff_files += 1;
        for c in &iff.chunks {
            let kind = c.kind();
            *report.chunks.entry(kind.clone()).or_default() += 1;
            let data = iff.data(c);
            let result = match &c.tag {
                b"BHAV" => Behavior::parse(data).map(|_| ()),
                b"DGRP" => DrawGroup::parse(data).map(|_| ()),
                b"PALT" => Palette::parse(data).map(|_| ()),
                b"TTAB" => InteractionTable::parse(data).map(|_| ()),
                b"TTAs" | b"STR#" | b"CTSS" => StringTable::parse(data).map(|_| ()),
                b"SPR2" => SpriteSet::parse(data).and_then(|set| {
                    if decode_sprites {
                        for n in 0..set.frames.len() {
                            set.decode(data, n, &iff)?;
                            report.decoded_frames += 1;
                        }
                    }
                    Ok(())
                }),
                _ => continue,
            };
            match result {
                Ok(()) => *report.typed_ok.entry(kind).or_default() += 1,
                Err(e) => report.failures.push(format!(
                    "{} :: {} #{} (chunk at {:#X}): {e}",
                    resource.locator, kind, c.id, c.offset
                )),
            }
        }
    }
    report
}
