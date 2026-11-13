//! 5835ed's ordered avatar search paths, with 415336 path constants and
//! 5c2f41/5c3136 normalization. File enumeration and resource precedence are
//! separate: the host must supply actual filesystem and object-provider state.
use super::generator::fixed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Appearances,
    Animations,
}

pub trait Host {
    fn exists(&mut self, path: &[u8]) -> Result<bool, String>;
    /// 43bdc5's unique directory vector, in its original resource order.
    fn providers(&mut self) -> Result<Vec<Vec<u8>>, String>;
    fn registered(&mut self, path: &[u8]) -> Result<bool, String>;
    fn register(&mut self, path: &[u8], recursive: bool, archives: bool) -> Result<(), String>;
    fn load(&mut self, kind: Kind, paths: &[Vec<u8>]) -> Result<(), String>;
}

/// 43ae81's registration segment, before object enumeration and avatar loading.
/// Prefix is copied directly, with no separator inserted. Registration uses
/// these raw paths; the object's later slash-normalization phase is separate.
pub fn initial_mounts(
    prefix: &[u8],
    expansions: i16,
    split_fars: bool,
    host: &mut impl Host,
) -> Result<Vec<Vec<u8>>, String> {
    let mut directories = vec![
        ("GameData/Global/".to_string(), true),
        ("GameData/Objects/".to_string(), true),
        ("GameData/UserObjects/".to_string(), true),
    ];
    for (bit, directory) in [
        (1, "ExpansionPack"),
        (2, "ExpansionPack2"),
        (4, "ExpansionPack3"),
        (8, "ExpansionPack4"),
        (16, "Deluxe"),
        (32, "ExpansionPack5"),
        (64, "ExpansionPack6"),
        (128, "ExpansionPack7"),
    ] {
        if expansions & bit != 0 {
            let suffix = if split_fars && bit != 64 {
                "Objects/"
            } else {
                ""
            };
            directories.push((format!("{directory}/{suffix}"), true));
        }
    }
    if expansions > 3 {
        directories.extend([
            ("ExpansionShared/".to_string(), true),
            ("TemplateNPCs/".to_string(), false),
        ]);
    }
    directories.push(("Downloads/".to_string(), true));
    let mut paths = vec![];
    for (suffix, recursive) in directories {
        let mut path = join(fixed(prefix, 259), suffix.as_bytes());
        path.truncate(259);
        if !host.registered(&path)? {
            host.register(&path, recursive, true)?;
        }
        paths.push(path);
    }
    Ok(paths)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub appearances: Vec<Vec<u8>>,
    pub animations: Vec<Vec<u8>>,
}
impl Paths {
    /// The initialized flag is published after both loads. The original tests
    /// a signed 16-bit expansion word for shared paths, and its low eight bits
    /// individually for expansion paths; these are not equivalent conditions.
    pub fn load_once(
        loaded: &mut Option<Self>,
        root: &[u8],
        expansions: i16,
        host: &mut impl Host,
    ) -> Result<(), String> {
        if loaded.is_some() {
            return Ok(());
        }
        let root = fixed(root, 259);
        let mut appearances = vec![join(root, b"Skins/")];
        for suffix in [b"cmx".to_vec()]
            .into_iter()
            .chain((2..=10).map(|n| format!("cmx{n}").into_bytes()))
            .chain(
                [
                    "cmxpreview",
                    "People1",
                    "People2",
                    "Accessories1",
                    "Accessories2",
                ]
                .map(|s| s.as_bytes().to_vec()),
            )
        {
            let path = join(root, &suffix);
            if host.exists(&path)? {
                appearances.push(path);
            }
        }
        if expansions > 3 {
            appearances.extend(
                [
                    "./ExpansionShared/Skins/",
                    "./ExpansionShared/SkinsBuy/",
                    "./ExpansionShared/Animations/",
                    "./ExpansionShared/Skeletons/",
                    "./TemplateNPCs/",
                    "./ExpansionShared/Accessories/",
                ]
                .map(|s| s.as_bytes().to_vec()),
            );
        }
        for (bit, directory) in [
            (0x80, "ExpansionPack7"),
            (0x40, "ExpansionPack6"),
            (0x20, "ExpansionPack5"),
            (0x10, "Deluxe"),
            (0x08, "ExpansionPack4"),
            (0x04, "ExpansionPack3"),
            (0x02, "ExpansionPack2"),
        ] {
            if expansions & bit != 0 {
                for suffix in ["Skins", "Animations", "Accessories"] {
                    appearances.push(format!("./{directory}/{suffix}/").into_bytes());
                }
            }
        }
        if expansions & 1 != 0 {
            appearances.extend([
                b"./ExpansionPack/".to_vec(),
                b"./ExpansionPack/ExpansionPackAssets/".to_vec(),
            ]);
        }
        let mut providers = host.providers()?;
        // The reverse erase loop preserves the forward order of the indexed
        // directories while appending unindexed appearance directories in reverse.
        for index in (0..providers.len()).rev() {
            let path = &providers[index];
            if path.is_empty() || path.len() >= 260 || !matches!(path.last(), Some(b'/' | b'\\')) {
                return Err(
                    "avatar provider directory is not a terminated CFString260 path".into(),
                );
            }
            let without_separator = &path[..path.len() - 1];
            let start = without_separator
                .iter()
                .rposition(|b| matches!(b, b'/' | b'\\' | b':'))
                .map_or(0, |n| n + 1);
            let name = &without_separator[start..];
            let end = name.iter().rposition(|b| *b == b'.').unwrap_or(name.len());
            // Copying the directory and appending its basename happen through
            // the original 259-byte bounded CFString storage.
            let mut ndx = join(path, &name[..end]);
            ndx.truncate(259);
            ndx.extend_from_slice(b".ndx");
            ndx.truncate(259);
            if !host.exists(&ndx)? {
                appearances.push(providers.remove(index));
            }
        }
        let appearances = resolver_paths(&appearances)?;
        load(host, Kind::Appearances, &appearances)?;

        let mut animations = vec![];
        let animation = join(root, b"Animation");
        if host.exists(&animation)? {
            animations.push(animation);
        }
        animations.extend(providers);
        let animations = resolver_paths(&animations)?;
        load(host, Kind::Animations, &animations)?;
        *loaded = Some(Self {
            appearances,
            animations,
        });
        Ok(())
    }
}

/// 413ccc's texture path phase, executed before 5835ed. Unlike the appearance
/// phase, it registers the raw installation-prefixed paths before normalizing
/// the final list. The returned list is published through 5cb046; textures are
/// enumerated later by 40895a. `providers` is the already-collected input vector.
pub fn texture_paths(
    prefix: &[u8],
    expansions: i16,
    providers: &[Vec<u8>],
    host: &mut impl Host,
) -> Result<Vec<Vec<u8>>, String> {
    let mut raw = vec![];
    let mut append = |directory: &str, subdirectory: &str, recursive| -> Result<(), String> {
        // 4ee8fe appends prefix, directory, a missing separator, then child,
        // applying CFString260 bounds separately at each append.
        let mut path = fixed(prefix, 259).to_vec();
        path.extend_from_slice(directory.as_bytes());
        path.truncate(259);
        if !matches!(path.last(), Some(b'/' | b'\\')) {
            path.push(b'/');
            path.truncate(259);
        }
        path.extend_from_slice(subdirectory.as_bytes());
        path.truncate(259);
        if !host.registered(&path)? {
            host.register(&path, recursive, true)?;
        }
        raw.push(path);
        Ok(())
    };
    if expansions > 3 {
        for subdirectory in [
            "ExpansionShared/Accessories/",
            "ExpansionShared/Skins/",
            "ExpansionShared/SkinsBuy/",
            "TemplateNPCs/",
        ] {
            append("", subdirectory, false)?;
        }
    }
    for (bit, directory) in [
        (0x80, "ExpansionPack7"),
        (0x40, "ExpansionPack6"),
        (0x20, "ExpansionPack5"),
        (0x10, "Deluxe"),
        (0x08, "ExpansionPack4"),
        (0x04, "ExpansionPack3"),
        (0x02, "ExpansionPack2"),
    ] {
        if expansions & bit != 0 {
            for child in ["Accessories", "Skins"] {
                append("", &format!("{directory}/{child}/"), false)?;
            }
        }
    }
    if expansions & 1 != 0 {
        append("", "ExpansionPack/ExpansionPackAssets/", false)?;
    }
    append("GameData/", "Skins/", true)?;
    append("GameData/", "Textures2/", false)?;
    append("GameData/", "Textures/", false)?;
    for provider in providers {
        if !host.registered(provider)? {
            host.register(provider, true, true)?;
        }
        raw.push(provider.clone());
    }
    resolver_paths(&raw)
}

fn join(a: &[u8], b: &[u8]) -> Vec<u8> {
    [a, b].concat()
}

fn load(host: &mut impl Host, kind: Kind, paths: &[Vec<u8>]) -> Result<(), String> {
    for path in paths {
        if !host.registered(path)? {
            host.register(path, true, true)?;
        }
    }
    host.load(kind, paths)
}

/// 5c2f41/5c3136: semicolon-separated search paths and native slash/root rules.
pub fn resolver_paths(values: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, String> {
    let joined = values.join(&b';');
    if joined.contains(&0) {
        return Err("avatar search path contains an embedded NUL".into());
    }
    let mut out = vec![];
    for (i, part) in joined.split(|b| *b == b';').enumerate() {
        if i == 0 && part.is_empty() && !joined.is_empty() {
            out.push(vec![]); // Native leading-semicolon empty entry is unnormalized.
            continue;
        }
        let part = if i == 0 {
            part
        } else {
            part.trim_ascii_start()
        };
        if part.len() >= 256 || (part.is_empty() && !joined.is_empty()) {
            return Err("avatar search path exceeds the native parser's supported bounds".into());
        }
        let mut path: Vec<_> = part
            .iter()
            .map(|&b| if b == b'/' { b'\\' } else { b })
            .collect();
        let absolute = path.len() > 2
            && ((path[0].is_ascii_alphabetic() && path[1..3] == *b":\\")
                || (path.starts_with(b"\\\\") && path[2].is_ascii_alphabetic()));
        if !(absolute || path.starts_with(b".\\") || path == b".") {
            path.splice(0..0, b".\\".iter().copied());
        }
        if !path.ends_with(b"\\") {
            path.push(b'\\');
        }
        out.push(path);
    }
    Ok(out)
}
