//! The ordered suit and texture names used by family import.
use crate::file_table::Table;
use crate::index::MountedFiles;
use crate::{Error, Expansions, Install, Resource, Result};
use sims_formats::avatar::catalog::Binary;
use sims_formats::avatar::index::{IndexReader, Kind};
use sims_formats::avatar::read_cmx;
use sims_formats::far::encode_name;
use sims_formats::iff::Iff;
use sims_formats::strings::decode;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Appearance inputs at startup for the Windows CD installation. Load once
/// and reuse for all calls to [`crate::family_import::refresh_outfits`].
#[derive(Debug, Clone)]
pub struct ImportWardrobe {
    pub(crate) suits: Vec<Vec<u8>>,
    pub(crate) textures: Vec<Vec<u8>>,
}

/// Mounted startup directories and the original's avatar search paths
/// (`0x43ae81`, `0x5835ed`): `appearance` holds the folders scanned eagerly
/// by `0x59f844`, `indexed` holds the folders whose `.ndx` catalogs are
/// registered by `0x584562` → `0x5a0212` → `0x59fc48`.
pub(crate) struct AvatarPaths {
    pub(crate) files: MountedFiles,
    pub(crate) appearance: Vec<String>,
    pub(crate) indexed: Vec<String>,
    pub(crate) texture_paths: Vec<String>,
}

/// Replays the startup file mounts (`0x43ae81`), download providers
/// (`0x43bdc5`), texture paths (`0x414633`) and avatar search paths
/// (`0x5835ed`). Shared by [`ImportWardrobe::load`] and
/// [`crate::people::AvatarRegistry`]. When `strict_archives` is false,
/// unreadable FAR archives are skipped during directory scanning as at
/// `0x5ba8d6`.
pub(crate) fn mount_avatar_paths(install: &Install, strict_archives: bool) -> Result<AvatarPaths> {
    let mut files = MountedFiles::default();
    let root = &install.root;
    for directory in ["GameData/Global", "GameData/Objects", Install::USER_OBJECTS] {
        files.register(root, directory, true, strict_archives)?;
    }
    for (bit, directory, _) in Expansions::ALL {
        if install.expansions.contains(bit) {
            files.register(root, directory, true, strict_archives)?;
        }
    }
    if install.expansions.0 > 3 {
        files.register(root, "ExpansionShared", true, strict_archives)?;
        files.register(root, "TemplateNPCs", false, strict_archives)?;
    }
    files.register(root, "Downloads", true, strict_archives)?;
    let providers = download_providers(&files, root)?;
    let mut texture_paths = texture_paths(&mut files, install, strict_archives)?;
    for path in &providers {
        files.register(&install.root, path, true, strict_archives)?;
    }
    texture_paths.extend(providers.iter().cloned());
    let (appearance, indexed) = appearance_paths(&mut files, install, providers, strict_archives)?;
    Ok(AvatarPaths {
        files,
        appearance,
        indexed,
        texture_paths,
    })
}

impl ImportWardrobe {
    /// Constructs an ordered wardrobe from explicit suit and texture lists
    /// (in exact native table/cache order), for synthetic tests.
    pub fn from_ordered_names(suits: Vec<Vec<u8>>, textures: Vec<Vec<u8>>) -> Self {
        Self { suits, textures }
    }

    /// The cached suit names (`0x6e48a0`), in Windows-1252 bytes.
    pub fn suits(&self) -> &[Vec<u8>] {
        &self.suits
    }

    /// The cached texture names (`0x6e48b0`), in Windows-1252 bytes.
    pub fn textures(&self) -> &[Vec<u8>] {
        &self.textures
    }

    /// Returns a copy of this wardrobe with its suit list replaced by the
    /// shared live `0x409255` (`0x6e48a0`) scan snapshot.
    pub fn with_suits(&self, suits: Vec<Vec<u8>>) -> Self {
        Self {
            suits,
            textures: self.textures.clone(),
        }
    }

    /// Replays the startup file tables (0x43ae81), avatar catalogs (0x5835ed)
    /// and texture cache (0x414633, 0x40895a).
    pub fn load(install: &Install) -> Result<Self> {
        let AvatarPaths {
            files,
            appearance,
            indexed,
            texture_paths,
        } = mount_avatar_paths(install, true)?;

        // 0x59f844 loads all CMX before BCF. 0x584562 then adds the indexed
        // names, including entries whose catalog has yet to be opened.
        let mut registry = Table::default();
        for extension in [b".cmx", b".bcf"] {
            for (_, source) in resources(&files, &appearance, extension)? {
                let bytes = source.read()?;
                let register = |name: &str, registry: &mut Table| {
                    let name = encode_name(name).ok_or_else(|| {
                        Error::Unsupported(format!(
                            "0x59f844 suit name is outside Windows-1252: {name}"
                        ))
                    })?;
                    registry.insert_name(name);
                    Ok(())
                };
                if extension == b".cmx" {
                    for suit in read_cmx(&bytes)
                        .map_err(|error| catalog_error(install, "0x594d73", &source, error))?
                        .appearances
                    {
                        register(&suit.name, &mut registry)?;
                    }
                } else {
                    let mut binary = Binary::new(&bytes);
                    for kind in [Kind::Skeleton, Kind::Appearance] {
                        let fail = |error| catalog_error(install, "0x595140", &source, error);
                        let Some(count) = binary.count(kind).map_err(fail)? else {
                            break;
                        };
                        for _ in 0..count {
                            let record = binary.read(kind).map_err(fail)?;
                            if kind == Kind::Appearance {
                                register(record.name(), &mut registry)?;
                            }
                        }
                    }
                }
            }
        }
        for (_, source) in resources(&files, &indexed, b".ndx")? {
            for entry in IndexReader::new(&source.read()?) {
                let entry =
                    entry.map_err(|error| catalog_error(install, "0x59fc48", &source, error))?;
                if entry.kind == Kind::Appearance {
                    registry.insert_name(entry.name);
                }
            }
        }
        let suits = registry
            .iter()
            // 0x409255 checks the lowercase prefix before the bounded copy.
            .filter(|name| !name.starts_with(b"suit-"))
            .map(|name| name[..name.len().min(63)].to_vec())
            .collect();

        let mut textures = Vec::new();
        for extension in [b".bmp", b".tga"] {
            textures.extend(
                resources(&files, &texture_paths, extension)?
                    .into_iter()
                    .map(|(mut name, _)| {
                        name.truncate(63);
                        name
                    }),
            );
        }
        Ok(Self { suits, textures })
    }

    /// As [`ImportWardrobe::load`], but skips unreadable FAR archives
    /// (`0x5ba8d6`) and records corrupt `.cmx`/`.bcf`/`.ndx` catalogs into the
    /// returned error map instead of failing boot.
    pub fn load_lenient(install: &Install) -> Result<(Self, BTreeMap<String, String>)> {
        let AvatarPaths {
            files,
            appearance,
            indexed,
            texture_paths,
        } = mount_avatar_paths(install, false)?;

        let mut registry = Table::default();
        let mut errors = BTreeMap::new();
        for extension in [b".cmx", b".bcf"] {
            for (_, source) in resources(&files, &appearance, extension)? {
                let location = source.location(&install.root);
                let bytes = match source.read() {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        errors.insert(location, error.to_string());
                        continue;
                    }
                };
                if extension == b".cmx" {
                    match read_cmx(&bytes) {
                        Ok(catalog) => {
                            for suit in catalog.appearances {
                                if let Some(name) = encode_name(&suit.name) {
                                    registry.insert_name(name);
                                }
                            }
                        }
                        Err(error) => {
                            errors.insert(location, error.to_string());
                        }
                    }
                } else {
                    let mut binary = Binary::new(&bytes);
                    for kind in [Kind::Skeleton, Kind::Appearance] {
                        let count = match binary.count(kind) {
                            Ok(Some(count)) => count,
                            Ok(None) => break,
                            Err(error) => {
                                errors.insert(location.clone(), error.to_string());
                                break;
                            }
                        };
                        let mut failed = false;
                        for _ in 0..count {
                            match binary.read(kind) {
                                Ok(record) => {
                                    if kind == Kind::Appearance
                                        && let Some(name) = encode_name(record.name())
                                    {
                                        registry.insert_name(name);
                                    }
                                }
                                Err(error) => {
                                    errors.insert(location.clone(), error.to_string());
                                    failed = true;
                                    break;
                                }
                            }
                        }
                        if failed {
                            break;
                        }
                    }
                }
            }
        }
        for (_, source) in resources(&files, &indexed, b".ndx")? {
            let location = source.location(&install.root);
            let bytes = match source.read() {
                Ok(bytes) => bytes,
                Err(error) => {
                    errors.insert(location, error.to_string());
                    continue;
                }
            };
            for entry in IndexReader::new(&bytes) {
                match entry {
                    Ok(entry) if entry.kind == Kind::Appearance => {
                        registry.insert_name(entry.name);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        errors.insert(location.clone(), error.to_string());
                    }
                }
            }
        }
        let suits = registry
            .iter()
            .filter(|name| !name.starts_with(b"suit-"))
            .map(|name| name[..name.len().min(63)].to_vec())
            .collect();

        let mut textures = Vec::new();
        for extension in [b".bmp", b".tga"] {
            textures.extend(
                resources(&files, &texture_paths, extension)?
                    .into_iter()
                    .map(|(mut name, _)| {
                        name.truncate(63);
                        name
                    }),
            );
        }
        Ok((Self { suits, textures }, errors))
    }

    /// Number of human body (`head == false`, `0x409663`) or head
    /// (`head == true`, `0x40a364`) entries in native wardrobe order.
    pub fn count(&self, head: bool, female: bool, child: bool, skin: &str) -> usize {
        let Some(skin_bytes) = encode_name(&skin.to_ascii_lowercase()) else {
            return 0;
        };
        let (first, length) = if head { (b'c', 6usize) } else { (b'b', 9usize) };
        let sex = if female { b'f' } else { b'm' };
        let age = if child { b'c' } else { b'a' };
        let mut total = 0usize;
        for suit in &self.suits {
            if suit.len() < length
                || !suit[0].eq_ignore_ascii_case(&first)
                || !(suit[4].eq_ignore_ascii_case(&sex) || suit[4].eq_ignore_ascii_case(&b'u'))
                || !suit[5].eq_ignore_ascii_case(&age)
                || (head && suit[1].eq_ignore_ascii_case(&b't'))
            {
                continue;
            }
            let mut prefix = suit[..length].to_ascii_lowercase();
            prefix.extend_from_slice(&skin_bytes);
            total += self
                .textures
                .iter()
                .filter(|t| {
                    t.len() >= prefix.len() && t[..prefix.len()].eq_ignore_ascii_case(&prefix)
                })
                .count();
        }
        total
    }

    /// Picks the `index`-th human body (`head == false`) or head (`head == true`)
    /// entry in native `0x409255`/`0x40895a` order, returning
    /// `(suit_name_63_lower, formatted_row)`.
    pub fn pick(
        &self,
        head: bool,
        female: bool,
        child: bool,
        skin: &str,
        mut index: usize,
    ) -> Option<(String, String)> {
        let skin_bytes = encode_name(&skin.to_ascii_lowercase())?;
        let (first, length, tag) = if head {
            (b'c', 6usize, "HEAD-HEAD")
        } else {
            (b'b', 9usize, "BODY")
        };
        let sex = if female { b'f' } else { b'm' };
        let age = if child { b'c' } else { b'a' };
        for suit in &self.suits {
            if suit.len() < length
                || !suit[0].eq_ignore_ascii_case(&first)
                || !(suit[4].eq_ignore_ascii_case(&sex) || suit[4].eq_ignore_ascii_case(&b'u'))
                || !suit[5].eq_ignore_ascii_case(&age)
                || (head && suit[1].eq_ignore_ascii_case(&b't'))
            {
                continue;
            }
            let mut prefix = suit[..length].to_ascii_lowercase();
            prefix.extend_from_slice(&skin_bytes);
            for tex in &self.textures {
                if tex.len() >= prefix.len() && tex[..prefix.len()].eq_ignore_ascii_case(&prefix) {
                    if index == 0 {
                        let suit_str: String = decode(suit, 1)
                            .to_ascii_lowercase()
                            .chars()
                            .take(63)
                            .collect();
                        let tex_str = decode(tex, 1);
                        // 0x409843/0x409851/0x472f41 caps the complete
                        // `BODY=<tex>` / `HEAD-HEAD=<tex>` descriptor at 63 bytes.
                        let desc_63: String = format!("{tag}={tex_str}").chars().take(63).collect();
                        let row = format!("{suit_str},{desc_63}");
                        return Some((suit_str, row));
                    }
                    index -= 1;
                }
            }
        }
        None
    }

    /// Number of pet body entries (`0x409663`) in native wardrobe order.
    pub fn count_pet(&self, cat: bool) -> usize {
        let species = if cat { b"cat" } else { b"dog" };
        let mut total = 0usize;
        for suit in &self.suits {
            if suit.len() < 7
                || !suit[0].eq_ignore_ascii_case(&b'b')
                || !suit[4..7].eq_ignore_ascii_case(species)
            {
                continue;
            }
            let prefix = &suit[..7];
            total += self
                .textures
                .iter()
                .filter(|t| t.len() >= 7 && t[..7].eq_ignore_ascii_case(prefix))
                .count();
        }
        total
    }

    /// Picks the `index`-th pet body entry (`0x409663`) in native
    /// `0x409255`/`0x40895a` order, returning
    /// `(suit_name_63_lower, formatted_row)`.
    pub fn pick_pet(&self, cat: bool, mut index: usize) -> Option<(String, String)> {
        let species = if cat { b"cat" } else { b"dog" };
        for suit in &self.suits {
            if suit.len() < 7
                || !suit[0].eq_ignore_ascii_case(&b'b')
                || !suit[4..7].eq_ignore_ascii_case(species)
            {
                continue;
            }
            let prefix = &suit[..7];
            for tex in &self.textures {
                if tex.len() >= 7 && tex[..7].eq_ignore_ascii_case(prefix) {
                    if index == 0 {
                        let suit_str: String = decode(suit, 1)
                            .to_ascii_lowercase()
                            .chars()
                            .take(63)
                            .collect();
                        let tex_str = decode(tex, 1);
                        let desc_63: String = format!("BODY={tex_str}").chars().take(63).collect();
                        let row = format!("{suit_str},{desc_63}");
                        return Some((suit_str, row));
                    }
                    index -= 1;
                }
            }
        }
        None
    }

    /// Whether the native-ordered human wardrobe list for `(head, female,
    /// child, skin)` contains an entry equal (case-insensitively on 63-byte
    /// suit and complete texture descriptor buffers, `0x472f92`) to `raw_row`,
    /// returning the matched 63-byte `(suit_lower, formatted_row)` when found.
    pub fn find_outfit(
        &self,
        head: bool,
        female: bool,
        child: bool,
        skin: &str,
        raw_row: &str,
    ) -> Option<(String, String)> {
        let row_255: String = raw_row.chars().take(255).collect();
        let before_semi = row_255.split(';').next().unwrap_or(&row_255);
        let (raw_suit, raw_tex) = before_semi.split_once(',')?;
        let row_suit: String = raw_suit.to_ascii_lowercase().chars().take(63).collect();
        let row_tex: String = raw_tex.to_ascii_lowercase().chars().take(63).collect();
        let skin_bytes = encode_name(&skin.to_ascii_lowercase())?;
        let (first, length, tag) = if head {
            (b'c', 6usize, "HEAD-HEAD")
        } else {
            (b'b', 9usize, "BODY")
        };
        let sex = if female { b'f' } else { b'm' };
        let age = if child { b'c' } else { b'a' };
        for suit in &self.suits {
            if suit.len() < length
                || !suit[0].eq_ignore_ascii_case(&first)
                || !(suit[4].eq_ignore_ascii_case(&sex) || suit[4].eq_ignore_ascii_case(&b'u'))
                || !suit[5].eq_ignore_ascii_case(&age)
                || (head && suit[1].eq_ignore_ascii_case(&b't'))
            {
                continue;
            }
            let suit_str: String = decode(suit, 1)
                .to_ascii_lowercase()
                .chars()
                .take(63)
                .collect();
            if suit_str != row_suit {
                continue;
            }
            let mut prefix = suit[..length].to_ascii_lowercase();
            prefix.extend_from_slice(&skin_bytes);
            for tex in &self.textures {
                if tex.len() >= prefix.len() && tex[..prefix.len()].eq_ignore_ascii_case(&prefix) {
                    let tex_str = decode(tex, 1);
                    let desc_63: String = format!("{tag}={tex_str}").chars().take(63).collect();
                    if desc_63.to_ascii_lowercase() == row_tex {
                        return Some((suit_str.clone(), format!("{suit_str},{desc_63}")));
                    }
                }
            }
        }
        None
    }
}

fn catalog_error(
    install: &Install,
    address: &str,
    source: &Resource,
    error: sims_formats::Error,
) -> Error {
    Error::format(
        &format!("{address} {}", source.location(&install.root)),
        error,
    )
}

/// At 0x43bdc5, the final registered OBJD for a resource contributes its
/// directory when category & 3 is 2. Only Downloads has that category
/// (0x43c609). A resource contributes once if any OBJD survived the +0x20
/// filter at registration, so its directory first appears in file walk order.
fn download_providers(files: &MountedFiles, root: &Path) -> Result<Vec<String>> {
    let mut providers = Vec::new();
    let mut seen = BTreeSet::new();
    for source in files.objects("Downloads")? {
        if !source.file_name().to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let fail = |error| Error::format(&format!("0x43b89c {}", source.location(root)), error);
        let iff = Iff::parse(source.read()?).map_err(fail)?;
        let definitions = iff.resource_order(b"OBJD").map_err(fail)?;
        if definitions.len() >= i16::MAX as usize {
            return Err(Error::Unsupported(format!(
                "0x43b89c OBJD ordinal overflow in {}",
                source.location(root)
            )));
        }
        let mut registered = false;
        for definition in definitions {
            let data = iff.data(definition);
            if data.len() < 4 {
                return Err(Error::Unsupported(format!(
                    "0x43b89c short OBJD in {}",
                    source.location(root)
                )));
            }
            // The definition loader zero-fills bytes past the original end.
            registered |= data.iter().skip(0x20).take(2).all(|&byte| byte == 0);
        }
        if !registered {
            continue;
        }
        let disk = source
            .path
            .strip_prefix(root)
            .expect("mounted below install");
        let disk = disk
            .to_str()
            .expect("validated Windows filename")
            .replace('\\', "/");
        let (parent, _) = disk.rsplit_once('/').expect("Downloads contains the file");
        let mut directory = parent.to_owned();
        if let Some(member) = source.member
            && let Some((inner, _)) = member.name.rsplit_once(['/', '\\'])
        {
            directory.push('/');
            directory.push_str(&inner.replace('\\', "/"));
        }
        directory.make_ascii_lowercase();
        if seen.insert(directory.clone()) {
            providers.push(directory);
        }
    }
    Ok(providers)
}

fn texture_paths(
    files: &mut MountedFiles,
    install: &Install,
    strict_archives: bool,
) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    if install.expansions.0 > 3 {
        paths.extend(
            [
                "ExpansionShared/Accessories",
                "ExpansionShared/Skins",
                "ExpansionShared/SkinsBuy",
                "TemplateNPCs",
            ]
            .map(str::to_owned),
        );
    }
    for (bit, directory, _) in Expansions::ALL.into_iter().skip(1).rev() {
        if install.expansions.contains(bit) {
            paths.extend(["Accessories", "Skins"].map(|child| format!("{directory}/{child}")));
        }
    }
    if install.expansions.contains(1) {
        paths.push("ExpansionPack/ExpansionPackAssets".into());
    }
    paths.extend(["GameData/Skins", "GameData/Textures2", "GameData/Textures"].map(str::to_owned));
    for path in &paths {
        files.register(
            &install.root,
            path,
            path == "GameData/Skins",
            strict_archives,
        )?;
    }
    Ok(paths)
}

fn appearance_paths(
    files: &mut MountedFiles,
    install: &Install,
    providers: Vec<String>,
    strict_archives: bool,
) -> Result<(Vec<String>, Vec<String>)> {
    let root = &install.root;
    let mut appearance = vec!["GameData/Skins".to_owned()];
    for name in ["cmx".to_owned()]
        .into_iter()
        .chain((2..=10).map(|i| format!("cmx{i}")))
        .chain(
            [
                "cmxpreview",
                "People1",
                "People2",
                "Accessories1",
                "Accessories2",
            ]
            .map(str::to_owned),
        )
    {
        let path = format!("GameData/{name}");
        if files.exists(root, &path)? {
            appearance.push(path);
        }
    }
    if install.expansions.0 > 3 {
        appearance.extend(
            [
                "ExpansionShared/Skins",
                "ExpansionShared/SkinsBuy",
                "ExpansionShared/Animations",
                "ExpansionShared/Skeletons",
                "TemplateNPCs",
                "ExpansionShared/Accessories",
            ]
            .map(str::to_owned),
        );
    }
    for (bit, directory, _) in Expansions::ALL.into_iter().skip(1).rev() {
        if install.expansions.contains(bit) {
            appearance.extend(
                ["Skins", "Animations", "Accessories"].map(|child| format!("{directory}/{child}")),
            );
        }
    }
    if install.expansions.contains(1) {
        appearance
            .extend(["ExpansionPack", "ExpansionPack/ExpansionPackAssets"].map(str::to_owned));
    }
    let mut indexed = Vec::new();
    if files.exists(root, "GameData/Animation")? {
        indexed.push("GameData/Animation".to_owned());
    }
    let mut loose = Vec::new();
    for path in providers {
        let name = path.rsplit('/').next().expect("provider name");
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        if files.exists(root, &format!("{path}/{stem}.ndx"))? {
            indexed.push(path);
        } else {
            loose.push(path);
        }
    }
    appearance.extend(loose.into_iter().rev());
    for path in appearance.iter().chain(&indexed) {
        files.register(root, path, true, strict_archives)?;
    }
    Ok((appearance, indexed))
}

/// 0x5c1a7d walks the file map of each path in precedence order. Within
/// each type the first occurrence of the same counted name and ID wins
/// (0x5c11d6); the extension remains part of equality across types.
pub(crate) fn resources_with_dir(
    files: &MountedFiles,
    paths: &[String],
    extension: &[u8],
) -> Result<Vec<(String, Vec<u8>, Resource)>> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for path in paths {
        for (filename, source) in files.files(path)? {
            let Some(mut name) = filename.strip_suffix(extension) else {
                continue;
            };
            let mut identity = 0x8000_0000;
            if let Some(prefix) = name.strip_prefix(b"_") {
                let end = prefix
                    .iter()
                    .position(|&byte| byte == b'_')
                    .filter(|&end| end <= 11)
                    .ok_or_else(|| {
                        Error::Unsupported(format!(
                            "0x5c17a3 invalid resource ID prefix: {}",
                            source.name()
                        ))
                    })?;
                identity = atoi(&prefix[..end]) as u32;
                name = &prefix[end + 1..];
            }
            if seen.insert((name.to_vec(), identity)) {
                result.push((path.clone(), name.to_vec(), source));
            }
        }
    }
    Ok(result)
}

fn resources(
    files: &MountedFiles,
    paths: &[String],
    extension: &[u8],
) -> Result<Vec<(Vec<u8>, Resource)>> {
    Ok(resources_with_dir(files, paths, extension)?
        .into_iter()
        .map(|(_, name, source)| (name, source))
        .collect())
}

// The original's C-locale atoi reads one signed decimal prefix.
pub(crate) fn atoi(text: &[u8]) -> i32 {
    let mut bytes = text
        .iter()
        .copied()
        .skip_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 11 | 12))
        .peekable();
    let negative = bytes.peek() == Some(&b'-');
    if matches!(bytes.peek(), Some(b'-' | b'+')) {
        bytes.next();
    }
    let mut value = 0i32;
    for digit in bytes.take_while(u8::is_ascii_digit) {
        value = value.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'));
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}
