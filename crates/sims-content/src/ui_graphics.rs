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

use crate::install::resolve;
use crate::{Error, Install, Result};
use sims_formats::bitmap::{self, Image};
use sims_formats::{far, ui_table};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

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
    archive: Option<PathBuf>,
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
        let root = install.file("UIGraphics")?;
        let mut ui = Self {
            root: install.root.clone(),
            archive: None,
            members: BTreeMap::new(),
            bitmaps: BTreeMap::new(),
            unresolved: Vec::new(),
        };
        if let Some(path) = resolve(&root, "UIGraphics.far")? {
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

    /// Reads a file by its path below `UIGraphics`.
    ///
    /// The path must stay below `UIGraphics` (no `..`, no absolute path),
    /// and the file must be at most 32 MiB.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let relative = format!("UIGraphics/{}", path.replace('\\', "/"));
        if !ui_table::valid_path(path) {
            return Err(Error::Unsupported(format!(
                "{relative} is not a path below UIGraphics"
            )));
        }
        let too_large = || Error::Unsupported(format!("{relative} is larger than 32 MiB"));
        if let Some(loose) = resolve(&self.root, &relative)?
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
        let stored = self.members.get(&normal(path)).ok_or_else(missing)?;
        let archive = self.archive.as_ref().ok_or_else(missing)?;
        let io = |e| Error::io(archive, e);
        let mut file = File::open(archive).map_err(io)?;
        file.seek(SeekFrom::Start(stored.offset)).map_err(io)?;
        if stored.size > MAX_FILE {
            return Err(too_large());
        }
        let size = stored.size as usize;
        let mut bytes = vec![0; size];
        file.read_exact(&mut bytes).map_err(io)?;
        Ok(bytes)
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
