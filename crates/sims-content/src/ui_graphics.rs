//! The interface's art: `UIGraphics/UIGraphics.far` and the loose files
//! beside it, and the resource tables that name its bitmaps by ID.
//!
//! The original's interface asks for a bitmap by resource ID. The four
//! `.RT` tables map each ID (per language) to a path below `UIGraphics`,
//! and the path is found as a loose file first, then as a member of the
//! archive, ignoring case and with `\` and `/` alike. The old prototype
//! looked paths up the same way
//! (legacy/crates/sims-content/src/dialog/ui_bitmaps.rs). **Unverified**:
//! that a loose file wins over an archive member. In this install the only
//! loose files are the tables and headers, and they are identical to the
//! archive's copies.

use crate::index::sorted_entries;
use crate::pool::ArchivePool;
use crate::{Error, Install, Result};
use sims_formats::bitmap::{self, Image};
use sims_formats::{far, ui_table};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The tables the interface reads, all in the archive's top level.
pub const TABLES: [&str; 4] = [
    "SMCtrlMgrRes.RT",
    "Res_CPanel.RT",
    "Res_Nbhd.RT",
    "Res_Other.RT",
];

/// The largest file read, and the largest archive directory: well above
/// the largest in the install (a 1024 by 768 loading screen of 769 KB).
const MAX_FILE: u64 = 32 << 20;

/// US English, the language the tests run in (`Res_Languages.h`).
pub const US_ENGLISH: u8 = 1;

/// A stored archive member.
#[derive(Debug, Clone)]
struct Stored {
    offset: u64,
    size: u64,
}

/// The UI archive, its loose files and its resource tables.
#[derive(Debug, Clone)]
pub struct UiGraphics {
    root: PathBuf,
    ui_root: PathBuf,
    archive: Option<PathBuf>,
    /// On-demand cache of sorted directory listings under `UIGraphics`,
    /// preserving exact component-by-component case resolution without touching
    /// unreferenced subdirectories or repeating `read_dir` on every bitmap.
    dir_cache: Arc<RwLock<BTreeMap<PathBuf, Vec<PathBuf>>>>,
    /// Cached open archive handles so reading hundreds of UI bitmaps avoids
    /// re-opening `UIGraphics.far` on every bitmap.
    pool: Arc<ArchivePool>,
    /// Members by lower-case path with `/` separators.
    members: BTreeMap<String, Stored>,
    /// Bitmap paths by (resource ID, language).
    bitmaps: BTreeMap<(i32, u8), String>,
    /// Rows naming constants no header declares (see
    /// [`ui_table::Table::unresolved`]).
    pub unresolved: Vec<String>,
}

fn normal(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

impl UiGraphics {
    /// Opens the install's `UIGraphics` and reads its four tables.
    pub fn open(install: &Install) -> Result<Self> {
        Self::open_with_pool(install, Arc::new(ArchivePool::new()))
    }

    /// Opens `UIGraphics` reusing open archive handles from `pool`.
    pub fn open_with_pool(install: &Install, pool: Arc<ArchivePool>) -> Result<Self> {
        let ui_root = install.file("UIGraphics")?;
        let mut ui = Self {
            root: install.root.clone(),
            ui_root,
            archive: None,
            dir_cache: Arc::new(RwLock::new(BTreeMap::new())),
            pool,
            members: BTreeMap::new(),
            bitmaps: BTreeMap::new(),
            unresolved: Vec::new(),
        };
        if let Some(path) = ui.resolve_cached("UIGraphics.far")? {
            ui.read_directory(path)?;
        }
        for name in TABLES {
            let text = ui.text(name)?;
            let location = |e| Error::format(&format!("UIGraphics/{name}"), e);
            // Each table resolves names with the headers it includes.
            let mut constants = BTreeMap::new();
            for header in &ui_table::includes(&text).map_err(location)? {
                ui_table::constants(&ui.text(header)?, &mut constants)
                    .map_err(|e| Error::format(&format!("UIGraphics/{header}"), e))?;
            }
            let table = ui_table::table(&text, &constants).map_err(location)?;
            for row in table.rows {
                let key = (row.id, row.language);
                if let Some(old) = ui.bitmaps.get(&key)
                    && !old.eq_ignore_ascii_case(&row.path)
                {
                    return Err(Error::Unsupported(format!(
                        "UIGraphics/{name}: resource {} (language {}) is both {old} and {}",
                        row.id, row.language, row.path
                    )));
                }
                ui.bitmaps.insert(key, row.path);
            }
            ui.unresolved.extend(table.unresolved);
        }
        Ok(ui)
    }

    fn read_directory(&mut self, path: PathBuf) -> Result<()> {
        let io = |e| Error::io(&path, e);
        let location = |e| Error::format(&path.display().to_string(), e);
        let mut file = File::open(&path).map_err(io)?;
        let length = file.metadata().map_err(io)?.len();
        let mut header = [0; 16];
        file.read_exact(&mut header).map_err(io)?;
        let offset = far::manifest_offset(&header, length).map_err(location)?;
        file.seek(SeekFrom::Start(u64::from(offset))).map_err(io)?;
        let mut directory = Vec::new();
        file.take(MAX_FILE)
            .read_to_end(&mut directory)
            .map_err(io)?;
        for entry in far::directory(&directory, length).map_err(location)? {
            // The first member of a name wins, as in a lookup that stops
            // at the first match.
            self.members.entry(normal(&entry.name)).or_insert(Stored {
                offset: u64::from(entry.offset),
                size: u64::from(entry.stored_size),
            });
        }
        self.archive = Some(path);
        Ok(())
    }

    fn cached_entries(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        if let Ok(guard) = self.dir_cache.read()
            && let Some(entries) = guard.get(dir)
        {
            return Ok(entries.clone());
        }
        let entries = sorted_entries(dir)?;
        if let Ok(mut guard) = self.dir_cache.write() {
            guard
                .entry(dir.to_path_buf())
                .or_insert_with(|| entries.clone());
        }
        Ok(entries)
    }

    /// Component-by-component case-insensitive path resolution under `ui_root`
    /// matching [`crate::install::resolve`], caching each visited directory's
    /// sorted listing on demand.
    fn resolve_cached(&self, relative: &str) -> Result<Option<PathBuf>> {
        let mut path = self.ui_root.clone();
        for component in relative.split('/').filter(|c| !c.is_empty()) {
            if component == "." || component == ".." {
                path.push(component);
                path.symlink_metadata().map_err(|e| Error::io(&path, e))?;
                continue;
            }
            let entries = self.cached_entries(&path)?;
            let name = |p: &PathBuf| {
                p.file_name()
                    .unwrap_or_default()
                    .as_encoded_bytes()
                    .to_vec()
            };
            let found = entries
                .iter()
                .find(|p| name(p) == component.as_bytes())
                .or_else(|| {
                    entries
                        .iter()
                        .find(|p| name(p).eq_ignore_ascii_case(component.as_bytes()))
                });
            match found {
                Some(next) => path = next.clone(),
                None => return Ok(None),
            }
        }
        Ok(Some(path))
    }

    /// Reads a file by its path below `UIGraphics`.
    ///
    /// The path must stay below `UIGraphics` (no `..`, no absolute path),
    /// and the file must be at most 32 MiB.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let rel_slash = path.replace('\\', "/");
        let relative = format!("UIGraphics/{rel_slash}");
        if !ui_table::valid_path(path) {
            return Err(Error::Unsupported(format!(
                "{relative} is not a path below UIGraphics"
            )));
        }
        let too_large = || Error::Unsupported(format!("{relative} is larger than 32 MiB"));
        if let Some(loose) = self.resolve_cached(&rel_slash)?
            && loose.is_file()
        {
            let io = |e| Error::io(&loose, e);
            let file = File::open(&loose).map_err(io)?;
            if file.metadata().map_err(io)?.len() > MAX_FILE {
                return Err(too_large());
            }
            let mut bytes = Vec::new();
            file.take(MAX_FILE).read_to_end(&mut bytes).map_err(io)?;
            return Ok(bytes);
        }
        let missing = || Error::Missing {
            root: self.root.clone(),
            path: relative.clone(),
        };
        let norm = normal(path);
        let stored = self.members.get(&norm).ok_or_else(missing)?;
        let archive = self.archive.as_ref().ok_or_else(missing)?;
        if stored.size > MAX_FILE {
            return Err(too_large());
        }
        self.pool.read_range(archive, stored.offset, stored.size)
    }

    fn text(&self, path: &str) -> Result<String> {
        Ok(sims_formats::strings::decode(&self.read(path)?, US_ENGLISH))
    }

    /// The path of resource `id` in `language`, falling back to the row
    /// for every language, as the old prototype did (0x5a578d).
    pub fn path(&self, id: i32, language: u8) -> Option<&str> {
        self.bitmaps
            .get(&(id, language))
            .or_else(|| self.bitmaps.get(&(id, 0)))
            .map(String::as_str)
    }

    /// The number of (ID, language) rows the tables give.
    pub fn len(&self) -> usize {
        self.bitmaps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bitmaps.is_empty()
    }

    /// Decodes the image at `path` below `UIGraphics`, by its extension.
    pub fn image(&self, path: &str) -> Result<Image> {
        let bytes = self.read(path)?;
        let location = |e| Error::format(&format!("UIGraphics/{path}"), e);
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".bmp") {
            bitmap::bmp(&bytes).map_err(location)
        } else if lower.ends_with(".tga") {
            bitmap::tga(&bytes).map_err(location)
        } else {
            Err(Error::Unsupported(format!(
                "UIGraphics/{path} is not a BMP or TGA image"
            )))
        }
    }

    /// Decodes resource `id` in `language`, or None when no table names it.
    pub fn bitmap(&self, id: i32, language: u8) -> Result<Option<Image>> {
        match self.path(id, language) {
            Some(path) => self.image(path).map(Some),
            None => Ok(None),
        }
    }
}
