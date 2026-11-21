//! The ordered suit and texture names used by family import.
use crate::file_table::Table;
use crate::index::MountedFiles;
use crate::{Error, Expansions, Install, Resource, Result};
use sims_formats::avatar::catalog::Binary;
use sims_formats::avatar::index::{IndexReader, Kind};
use sims_formats::avatar::read_cmx;
use sims_formats::far::encode_name;
use sims_formats::iff::Iff;
use std::collections::BTreeSet;
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
