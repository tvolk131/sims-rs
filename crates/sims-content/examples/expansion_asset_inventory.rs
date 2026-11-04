//! Read-only physical corpus and effective startup-catalog audit.
//! Metadata only: never exports copyrighted resource payloads or runs the VM.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_content::{Index, Resource, avatar_installation, object_catalog};
use sims_formats::{
    animation,
    avatar::{self, catalog::Asset, index::Kind},
    behavior::{Behavior, InteractionTable, ObjectDefinition},
    iff::Iff,
    sprite::{DrawGroup, LegacySpriteSet, Palette, SpriteSet},
    strings::StringTable,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

#[derive(Debug, Default, Serialize)]
struct Stats {
    resources: usize,
    resource_bytes: u64,
    archives: BTreeSet<String>,
    extensions: BTreeMap<String, usize>,
    containers: usize,
    chunks: BTreeMap<String, usize>,
    parsed: BTreeMap<String, usize>,
    frames: BTreeMap<String, usize>,
    variants: BTreeMap<String, BTreeMap<String, usize>>,
    audio_prefixes: BTreeMap<String, BTreeMap<String, usize>>,
}
#[derive(Debug, Serialize)]
struct Issue {
    source: String,
    component: String,
    detail: String,
}
#[derive(Debug, Serialize)]
struct Link {
    source: String,
    component: String,
    target: String,
    candidates: Vec<String>,
    /// This is physical dependency locality, not native override order.
    local_candidates: Vec<String>,
    status: String,
}
#[derive(Default)]
struct Audit {
    groups: BTreeMap<String, Stats>,
    issues: Vec<Issue>,
    links: Vec<Link>,
    hashes: BTreeMap<String, String>,
    assets: BTreeMap<usize, BTreeSet<(usize, String)>>,
    animations: Vec<(usize, animation::Animation)>,
    appearances: Vec<(usize, avatar::Appearance)>,
    meshes: Vec<(usize, avatar::Mesh)>,
    indexes: Vec<(usize, avatar::index::Entry)>,
    guids: BTreeMap<u32, Vec<Value>>,
    languages: BTreeMap<u8, usize>,
    string_replacements: Vec<Value>,
    decoded_samples: usize,
}
fn group(resource: &Resource) -> &str {
    resource.locator.split('/').next().unwrap_or("<root>")
}
fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map_or("<none>", |(_, e)| e)
        .to_ascii_lowercase()
}
fn issue(audit: &mut Audit, resource: &Resource, component: &str, error: impl std::fmt::Display) {
    audit.issues.push(Issue {
        source: resource.locator.clone(),
        component: component.into(),
        detail: error.to_string(),
    });
}
fn variant(stats: &mut Stats, kind: &str, value: impl ToString) {
    *stats
        .variants
        .entry(kind.into())
        .or_default()
        .entry(value.to_string())
        .or_default() += 1;
}
fn prefix(resource: &Resource) -> Result<Vec<u8>> {
    let mut file = File::open(&resource.source)?;
    ensure!(
        resource.size == resource.unpacked_size,
        "compressed FAR entry"
    );
    ensure!(
        resource
            .offset
            .checked_add(resource.size)
            .is_some_and(|end| end <= file.metadata().map(|m| m.len()).unwrap_or(0)),
        "resource outside source"
    );
    file.seek(SeekFrom::Start(resource.offset))?;
    let mut bytes = vec![0; resource.size.min(64) as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn audio_prefix(bytes: &[u8]) -> String {
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        if bytes.get(12..16) == Some(b"fmt ") && bytes.len() >= 24 {
            return format!(
                "RIFF WAVE format {}",
                u16::from_le_bytes(bytes[20..22].try_into().unwrap())
            );
        }
        return "RIFF WAVE, fmt not in fixed prefix".into();
    }
    if bytes.starts_with(b"ID3") {
        return "MPEG audio with ID3".into();
    }
    if bytes.first() == Some(&0xff) && bytes.get(1).is_some_and(|b| b & 0xe0 == 0xe0) {
        return "MPEG frame sync".into();
    }
    if bytes.starts_with(b"XA\0\0") {
        return "XA00".into();
    }
    for signature in [b"XAI".as_slice(), b"XAJ", b"UTM0", b"TKDT", b"HIT!"] {
        if bytes.starts_with(signature) {
            return String::from_utf8_lossy(signature).into_owned();
        }
    }
    format!(
        "prefix:{}",
        bytes
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}
fn catalog(bytes: &[u8], text: bool) -> Result<Vec<Asset>> {
    let mut output = vec![];
    let mut binary = avatar::catalog::Binary::new(bytes);
    let mut textual = if text {
        Some(avatar::Cmx::new(bytes)?)
    } else {
        None
    };
    for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
        let count = if let Some(r) = &mut textual {
            r.count(kind)?
        } else {
            binary.count(kind)?
        };
        let count = count.context("catalog count short read")?;
        for _ in 0..count.max(0) {
            output.push(if let Some(r) = &mut textual {
                r.read(kind)?
            } else {
                binary.read(kind)?
            });
        }
    }
    Ok(output)
}
fn iff_audit(audit: &mut Audit, resource: &Resource, iff: &Iff) {
    audit.groups.get_mut(group(resource)).unwrap().containers += 1;
    for chunk in &iff.chunks {
        let tag = chunk.kind();
        let data = iff.data(chunk);
        let stats = audit.groups.get_mut(group(resource)).unwrap();
        *stats.chunks.entry(tag.clone()).or_default() += 1;
        if matches!(&chunk.tag, b"STR#" | b"CTSS" | b"TTAs") && data.len() >= 2 {
            variant(
                stats,
                &tag,
                i16::from_le_bytes(data[..2].try_into().unwrap()),
            );
        }
        if chunk.tag == *b"BHAV" && data.len() >= 2 {
            variant(
                stats,
                &tag,
                u16::from_le_bytes(data[..2].try_into().unwrap()),
            );
        }
        if chunk.tag == *b"TTAB" && data.len() >= 2 {
            if data[..2] == [0, 0] {
                variant(stats, &tag, "empty");
            } else if data.len() >= 4 {
                variant(
                    stats,
                    &tag,
                    u16::from_le_bytes(data[2..4].try_into().unwrap()),
                );
            }
        }
        if matches!(&chunk.tag, b"OBJD" | b"SPR2" | b"DGRP") && data.len() >= 4 {
            variant(
                stats,
                &tag,
                u32::from_le_bytes(data[..4].try_into().unwrap()),
            );
        }
        if chunk.tag == *b"SPR#" && data.len() >= 4 {
            let word = u32::from_le_bytes(data[..4].try_into().unwrap());
            variant(
                stats,
                &tag,
                if (500..=506).contains(&word.swap_bytes()) {
                    word.swap_bytes()
                } else {
                    word
                },
            );
        }
        let component = format!("{tag} #{}", chunk.id);
        let result: Result<()> = (|| {
            match &chunk.tag {
                b"BHAV" => {
                    Behavior::parse(data)?;
                }
                b"OBJD" => {
                    let o = ObjectDefinition::parse(data)?;
                    audit
                        .guids
                        .entry(o.guid)
                        .or_default()
                        .push(json!({"source":resource.locator,"chunk":chunk.id}));
                }
                b"TTAB" => {
                    InteractionTable::parse(data)?;
                }
                b"DGRP" => {
                    DrawGroup::parse(data)?;
                }
                b"PALT" => {
                    Palette::parse(data)?;
                }
                b"STR#" | b"CTSS" | b"TTAs" => {
                    let strings = StringTable::parse(data)?;
                    for entry in strings.entries {
                        *audit.languages.entry(entry.language).or_default() += 1;
                        if entry.text.contains('\u{fffd}') || entry.comment.contains('\u{fffd}') {
                            audit.string_replacements.push(json!({"source":resource.locator,"chunk":chunk.id,"language":entry.language}));
                        }
                    }
                }
                b"SPR2" => {
                    let set = SpriteSet::parse(data)?;
                    for i in 0..set.frames.len() {
                        set.decode(data, i, iff)?;
                    }
                    *audit
                        .groups
                        .get_mut(group(resource))
                        .unwrap()
                        .frames
                        .entry(tag.clone())
                        .or_default() += set.frames.len();
                }
                b"SPR#" => {
                    let set = LegacySpriteSet::parse(data)?;
                    for i in 0..set.frames.len() {
                        if let Err(error) = set.decode(data, i, iff) {
                            if error
                                .to_string()
                                .contains("requires an explicit engine default palette")
                            {
                                // Structural validation only. This cannot establish the actual
                                // colors without the original caller's palette input.
                                for f in 0..set.frames.len() {
                                    set.decode_with_palette(
                                        data,
                                        f,
                                        &Palette(vec![[255; 3]; 256]),
                                    )?;
                                }
                                let stats = audit.groups.get_mut(group(resource)).unwrap();
                                *stats
                                    .parsed
                                    .entry("SPR# structural with supplied palette".into())
                                    .or_default() += 1;
                                *stats
                                    .frames
                                    .entry("SPR# structural with supplied palette".into())
                                    .or_default() += set.frames.len();
                            }
                            return Err(error.into());
                        }
                    }
                    *audit
                        .groups
                        .get_mut(group(resource))
                        .unwrap()
                        .frames
                        .entry(tag.clone())
                        .or_default() += set.frames.len();
                }
                _ => return Ok(()),
            }
            *audit
                .groups
                .get_mut(group(resource))
                .unwrap()
                .parsed
                .entry(tag.clone())
                .or_default() += 1;
            Ok(())
        })();
        if let Err(error) = result {
            issue(audit, resource, &component, format!("{error:#}"));
        }
    }
}
fn inventory(index: &Index) -> Result<Audit> {
    let mut audit = Audit::default();
    for (id, resource) in index.resources.iter().enumerate() {
        let ext = extension(&resource.name);
        let stats = audit.groups.entry(group(resource).into()).or_default();
        stats.resources += 1;
        stats.resource_bytes += resource.size;
        *stats.extensions.entry(ext.clone()).or_default() += 1;
        if resource.archived {
            stats
                .archives
                .insert(resource.locator.split_once("::").unwrap().0.into());
        }
        let first = match prefix(resource) {
            Ok(first) => first,
            Err(error) => {
                issue(&mut audit, resource, "prefix", format!("{error:#}"));
                continue;
            }
        };
        if matches!(
            ext.as_str(),
            "wav" | "mp3" | "xa" | "utk" | "hit" | "hot" | "hsm" | "fsc"
        ) {
            *audit
                .groups
                .get_mut(group(resource))
                .unwrap()
                .audio_prefixes
                .entry(ext.clone())
                .or_default()
                .entry(audio_prefix(&first))
                .or_default() += 1;
        }
        let is_iff = first.starts_with(b"IFF FILE 2.5:") || first.starts_with(b"IFF FILE 2.0:");
        if !is_iff
            && !matches!(
                ext.as_str(),
                "bcf" | "cmx" | "ndx" | "bmf" | "skn" | "bmp" | "tga"
            )
        {
            continue;
        }
        let bytes = match resource.read() {
            Ok(bytes) => bytes,
            Err(error) => {
                issue(&mut audit, resource, "read", format!("{error:#}"));
                continue;
            }
        };
        audit.hashes.insert(
            resource.locator.clone(),
            format!("{:x}", Sha256::digest(&bytes)),
        );
        let result: Result<()> = (|| {
            if is_iff {
                iff_audit(&mut audit, resource, &Iff::parse(bytes)?);
            } else {
                match ext.as_str() {
                    "bcf" | "cmx" => {
                        for asset in catalog(&bytes, ext == "cmx")? {
                            audit
                                .assets
                                .entry(id)
                                .or_default()
                                .insert((asset.kind().index(), asset.name().to_ascii_lowercase()));
                            let label = match asset {
                                Asset::Skeleton(_) => "skeletons",
                                Asset::Appearance(ref a) => {
                                    audit.appearances.push((id, a.clone()));
                                    "appearances"
                                }
                                Asset::Animation(ref a) => {
                                    audit.animations.push((id, a.clone()));
                                    "animations"
                                }
                            };
                            *audit
                                .groups
                                .get_mut(group(resource))
                                .unwrap()
                                .parsed
                                .entry(label.into())
                                .or_default() += 1;
                        }
                    }
                    "ndx" => {
                        let mut reader = avatar::index::IndexReader::new(&bytes);
                        for entry in reader.by_ref() {
                            audit.indexes.push((id, entry?));
                        }
                        let trailing = bytes.len() - reader.bytes_consumed();
                        if trailing != 0 {
                            issue(&mut audit, resource, "NDX trailing data", trailing);
                        }
                    }
                    "bmf" | "skn" => {
                        let mesh = if ext == "bmf" {
                            avatar::read_bmf(&bytes)?
                        } else {
                            avatar::read_skn(&bytes)?
                        };
                        audit.meshes.push((id, mesh));
                    }
                    "bmp" | "tga" => {
                        if ext == "bmp" && bytes.len() >= 34 {
                            let s = audit.groups.get_mut(group(resource)).unwrap();
                            variant(
                                s,
                                "BMP.bits_per_pixel",
                                u16::from_le_bytes(bytes[28..30].try_into().unwrap()),
                            );
                            variant(
                                s,
                                "BMP.compression",
                                u32::from_le_bytes(bytes[30..34].try_into().unwrap()),
                            );
                        }
                        let mut reader = image::ImageReader::with_format(
                            Cursor::new(&bytes),
                            if ext == "bmp" {
                                image::ImageFormat::Bmp
                            } else {
                                image::ImageFormat::Tga
                            },
                        );
                        let mut limits = image::Limits::default();
                        limits.max_alloc = Some(128 * 1024 * 1024);
                        reader.limits(limits);
                        reader.decode()?;
                    }
                    _ => unreachable!(),
                }
                *audit
                    .groups
                    .get_mut(group(resource))
                    .unwrap()
                    .parsed
                    .entry(ext.clone())
                    .or_default() += 1;
            }
            Ok(())
        })();
        if let Err(error) = result {
            issue(&mut audit, resource, &ext, format!("{error:#}"));
        }
    }
    Ok(audit)
}

fn locality(resource: &Resource) -> String {
    if let Some((archive, member)) = resource.locator.split_once("::") {
        format!(
            "{archive}::{}",
            member.rsplit_once(['/', '\\']).map_or("", |(dir, _)| dir)
        )
    } else {
        resource
            .locator
            .rsplit_once('/')
            .map_or("", |(dir, _)| dir)
            .to_owned()
    }
}
fn candidates(
    index: &Index,
    by_name: &BTreeMap<String, Vec<usize>>,
    source: usize,
    names: &[String],
) -> (Vec<usize>, Vec<usize>) {
    let all: BTreeSet<_> = names
        .iter()
        .flat_map(|name| {
            by_name
                .get(&name.to_ascii_lowercase())
                .into_iter()
                .flatten()
                .copied()
        })
        .collect();
    let local = all
        .iter()
        .copied()
        .filter(|id| locality(&index.resources[*id]) == locality(&index.resources[source]))
        .collect();
    (all.into_iter().collect(), local)
}
fn links(index: &Index, audit: &mut Audit) -> Result<()> {
    let mut by_name = BTreeMap::<String, Vec<usize>>::new();
    for (id, resource) in index.resources.iter().enumerate() {
        by_name
            .entry(resource.name.to_ascii_lowercase())
            .or_default()
            .push(id);
        if let Some((_, leaf)) = resource.name.rsplit_once(['/', '\\']) {
            by_name
                .entry(leaf.to_ascii_lowercase())
                .or_default()
                .push(id);
        }
    }
    let mut pending = vec![];
    for (source, entry) in &audit.indexes {
        let target = format!("{}.bcf", sims_formats::strings::decode(&entry.catalog, 1));
        let (all, local) = candidates(index, &by_name, *source, std::slice::from_ref(&target));
        let selected = if !local.is_empty() { &local } else { &all };
        let found = selected
            .iter()
            .filter(|id| {
                audit.assets.get(id).is_some_and(|entries| {
                    entries.contains(&(
                        entry.kind.index(),
                        sims_formats::strings::decode(&entry.name, 1).to_ascii_lowercase(),
                    ))
                })
            })
            .count();
        let status = if all.is_empty() {
            "missing-file"
        } else if found == 0 {
            "record-not-decoded"
        } else if selected.len() == 1 {
            "physical-record-found"
        } else {
            "multiple-physical-providers"
        };
        pending.push((
            *source,
            format!(
                "NDX {:?}: {}",
                entry.kind,
                sims_formats::strings::decode(&entry.name, 1)
            ),
            target,
            all,
            local,
            status.to_owned(),
        ));
    }
    for (source, appearance) in &audit.appearances {
        for binding in &appearance.bindings {
            let names = [
                format!("{}.bmf", binding.mesh),
                format!("{}.skn", binding.mesh),
                binding.mesh.clone(),
            ];
            let (all, local) = candidates(index, &by_name, *source, &names);
            pending.push((
                *source,
                format!("appearance {} mesh", appearance.name),
                binding.mesh.clone(),
                all,
                local,
                String::new(),
            ));
        }
    }
    for (source, animation) in &audit.animations {
        let target = format!("{}.cfp", animation.skill);
        let (all, local) = candidates(index, &by_name, *source, std::slice::from_ref(&target));
        let selected = if !local.is_empty() { &local } else { &all };
        for &id in selected {
            let resource = &index.resources[id];
            match resource.read().and_then(|data| {
                audit.hashes.insert(
                    resource.locator.clone(),
                    format!("{:x}", Sha256::digest(&data)),
                );
                Ok(animation::read_cfp(
                    &data,
                    animation.translation_count,
                    animation.rotation_count,
                )?)
            }) {
                Ok(_) => {
                    audit.decoded_samples += 1;
                }
                Err(error) => audit.issues.push(Issue {
                    source: resource.locator.clone(),
                    component: format!("CFP for {}", animation.name),
                    detail: format!("{error:#}"),
                }),
            }
        }
        pending.push((
            *source,
            format!("animation {} samples", animation.name),
            target,
            all,
            local,
            String::new(),
        ));
    }
    for (source, mesh) in &audit.meshes {
        let (all, local, status) =
            if sims_content::avatar_material::builtin(&mesh.texture).is_some() {
                (vec![], vec![], "builtin-material".to_owned())
            } else if mesh.texture == "x" || mesh.texture.is_empty() {
                (vec![], vec![], "caller-supplied-texture".to_owned())
            } else {
                let (all, local) = candidates(
                    index,
                    &by_name,
                    *source,
                    &[
                        format!("{}.bmp", mesh.texture),
                        format!("{}.tga", mesh.texture),
                    ],
                );
                (all, local, String::new())
            };
        pending.push((
            *source,
            format!("mesh {} material", mesh.name),
            mesh.texture.clone(),
            all,
            local,
            status,
        ));
    }
    for (source, component, target, all, local, status) in pending {
        let selected = if !local.is_empty() { &local } else { &all };
        let status = if !status.is_empty() {
            status
        } else if selected.is_empty() {
            "missing-file".into()
        } else if selected.len() == 1 {
            "physical-file-found".into()
        } else if selected
            .iter()
            .filter_map(|id| audit.hashes.get(&index.resources[*id].locator))
            .collect::<BTreeSet<_>>()
            .len()
            == 1
            && selected
                .iter()
                .all(|id| audit.hashes.contains_key(&index.resources[*id].locator))
        {
            "identical-physical-providers".into()
        } else {
            "multiple-physical-providers".into()
        };
        audit.links.push(Link {
            source: index.resources[source].locator.clone(),
            component,
            target,
            candidates: all
                .iter()
                .map(|id| index.resources[*id].locator.clone())
                .collect(),
            local_candidates: local
                .iter()
                .map(|id| index.resources[*id].locator.clone())
                .collect(),
            status,
        });
    }
    Ok(())
}

fn startup(index: Arc<Index>) -> Result<Value> {
    let mut objects = object_catalog::Catalog::household(
        index.clone(),
        "UserData/Neighborhood.iff",
        Default::default(),
    )?;
    let providers = objects.providers().map_err(anyhow::Error::msg)?;
    let mut bindings = avatar_installation::complete_collection_asset_bindings(&index.root);
    bindings.push(avatar_installation::Binding {
        directory: b"./UserData/Characters/".to_vec(),
        physical: index.root.join("UserData/Characters"),
    });
    let mut installation = avatar_installation::Installation::new(&index, bindings)?;
    let initial = installation.initial_mounts(b"./", 255, false)?;
    installation.register(b"./UserData/Characters/", false, true)?;
    let loaded = installation.load_catalogs(b"./", b"./GameData/", 255, &providers)?;
    let directories = installation.directories()?;
    let mut registered = vec![];
    for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
        for row in loaded.catalog.iter(kind) {
            let source = loaded
                .catalog
                .asset(kind, &row.location.name)
                .map(|a| a.source);
            let indexed = loaded.catalog.index_source(kind, &row.location.name);
            let lazy_source = if row.attempted {
                None
            } else {
                Some(
                    loaded
                        .catalog
                        .catalog_source(&directories, kind, &row.location.name)
                        .map(|id| index.resources[id].locator.clone())
                        .map_err(|e| e.to_string()),
                )
            };
            registered.push(json!({"kind":format!("{kind:?}"),"name":sims_formats::strings::decode(&row.location.name,1),"directory":String::from_utf8_lossy(&row.location.directory),"catalog":String::from_utf8_lossy(&row.location.catalog),"context":row.location.context,"attempted":row.attempted,"decoded_source":source.map(|id|&index.resources[id].locator),"index_source":indexed.map(|id|&index.resources[id].locator),"lazy_source":lazy_source}));
        }
    }
    let mut definitions = vec![];
    for e in &objects.definitions.entries {
        definitions.push(json!({"index":e.index,"guid":u32::from_le_bytes(e.data[28..32].try_into().unwrap()),"resource":e.key.resource,"chunk":e.key.chunk,"path":String::from_utf8_lossy(&objects.resources.filename(e.key.resource).map_err(anyhow::Error::msg)?)}));
    }
    let strings = |rows: &[Vec<u8>]| {
        rows.iter()
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect::<Vec<_>>()
    };
    Ok(
        json!({"scope":"explicit Complete Collection asset bindings, expansion word255, UserData neighborhood; adapter filesystem order is ASCII-insensitive, not observed Wine enumeration","initial_mounts":strings(&initial),"object_visits":objects.visits,"definitions":definitions,"providers":strings(&providers),"appearance_paths":strings(&loaded.paths.appearances),"animation_paths":strings(&loaded.paths.animations),"texture_paths":strings(&loaded.textures),"startup_visits":loaded.visits,"registered":registered,"read_failures":loaded.catalog.read_failures.iter().map(|id|&index.resources[*id].locator).collect::<Vec<_>>(),"index_read_failures":loaded.catalog.index_read_failures.iter().map(|id|&index.resources[*id].locator).collect::<Vec<_>>(),"startup_open_failures":format!("{:?}",loaded.catalog.startup_open_failures)}),
    )
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: expansion_asset_inventory CONTENT_ROOT NEW_PRIVATE_OUTPUT_DIRECTORY"
    );
    let out = Path::new(&args[1]);
    ensure!(
        out.parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private cache output required"
    );
    fs::create_dir(out)?;
    let index = Arc::new(Index::scan(Path::new(&args[0]))?);
    let mut audit = inventory(&index)?;
    links(&index, &mut audit)?;
    let startup =
        startup(index.clone()).unwrap_or_else(|error| json!({"error":format!("{error:#}")}));
    let mut link_counts = BTreeMap::<&str, usize>::new();
    for link in &audit.links {
        *link_counts.entry(&link.status).or_default() += 1;
    }
    let report = json!({"schema":1,"source_root":index.root,"indexed_resources":index.resources.len(),"archives":index.archives,"index_diagnostics":index.diagnostics,"groups":audit.groups,"decode_issues":audit.issues,"decoded_cfp_references":audit.decoded_samples,"physical_dependency_counts":link_counts,"physical_dependencies":audit.links,"language_entries":audit.languages,"decoded_string_replacement_characters":audit.string_replacements,"duplicate_object_guids":audit.guids.iter().filter(|(_,rows)|rows.len()>1).map(|(guid,rows)|json!({"guid":guid,"providers":rows})).collect::<Vec<_>>(),"caveats":["Physical groups are extraction provenance, not the effective runtime expansion configuration.","A decoded file, registered resource or absent static reference is not evidence of reachable gameplay support.","Static dependency locality is diagnostic only: it does not substitute for native path order, aliases, lazy loading, fallback or caller-selected material/texture.","Parser failures are current Rust support/validation boundaries, not proof the original game rejects the file.","SPR# decode uses embedded palettes; original engine-supplied palettes may intentionally be absent from some mask resources.","Non-image/audio payloads are signature-counted only unless explicitly listed under parsed." ]});
    fs::write(
        out.join("physical.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        out.join("startup.json"),
        serde_json::to_vec_pretty(&startup)?,
    )?;
    fs::write(
        out.join("hashes.json"),
        serde_json::to_vec_pretty(&audit.hashes)?,
    )?;
    let ranges = index.resources.iter().filter(|r| audit.hashes.contains_key(&r.locator)).map(|r| json!({
        "locator": r.locator, "source": r.source.strip_prefix(&index.root).unwrap_or(&r.source),
        "offset": r.offset, "size": r.size, "unpacked_size": r.unpacked_size,
        "archived": r.archived, "sha256": audit.hashes[&r.locator],
    })).collect::<Vec<_>>();
    fs::write(
        out.join("resource-ranges.json"),
        serde_json::to_vec_pretty(&ranges)?,
    )?;
    let animations = audit
        .animations
        .iter()
        .map(|(id, a)| {
            json!({
                "source": index.resources[*id].locator, "name": a.name, "skill": a.skill,
                "translation_count": a.translation_count, "rotation_count": a.rotation_count,
                "frames": a.frames, "duration_ms": a.duration_ms, "motions": a.motions.len(),
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        out.join("animation-records.json"),
        serde_json::to_vec_pretty(&animations)?,
    )?;
    let meshes = audit.meshes.iter().map(|(id, m)| json!({
        "source": index.resources[*id].locator, "name": m.name, "texture": m.texture,
        "vertices": m.vertices.len(), "triangles": m.triangles.len(), "bones": m.bones.len(),
    })).collect::<Vec<_>>();
    fs::write(
        out.join("mesh-records.json"),
        serde_json::to_vec_pretty(&meshes)?,
    )?;
    println!(
        "{} resources, {} archives, {} decode diagnostics, {} physical links; {}",
        index.resources.len(),
        index.archives,
        audit.issues.len(),
        audit.links.len(),
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resource(locator: &str) -> Resource {
        Resource {
            locator: locator.into(),
            name: locator.rsplit('/').next().unwrap().into(),
            source: locator.into(),
            offset: 0,
            size: 0,
            unpacked_size: 0,
            archived: locator.contains("::"),
            owned: None,
        }
    }
    #[test]
    fn physical_group_and_locality_keep_archive_identity() {
        let r = resource("ExpansionPack7/Magic.far::Skins\\hat.cmx.bcf");
        assert_eq!(group(&r), "ExpansionPack7");
        assert_eq!(locality(&r), "ExpansionPack7/Magic.far::Skins");
        assert_eq!(extension("Hat.CMX.BCF"), "bcf");
        assert_ne!(
            locality(&r),
            locality(&resource("ExpansionPack6/Magic.far::Skins\\hat.cmx.bcf"))
        );
    }
    #[test]
    fn binary_catalog_failure_is_not_silent_success() {
        assert!(catalog(&[0; 8], false).is_err());
        assert!(catalog(&[0; 12], false).unwrap().is_empty());
    }
    #[test]
    fn empty_skn_texture_currently_shifts_the_count_into_the_name() {
        // Synthetic zero-vertex mesh. This records the current boundary; it
        // deliberately does not assert what the original accepts for an empty name.
        let control = b"audit-mesh\r\nx\r\n1\r\nROOT\r\n0\r\n0\r\n0\r\n0\r\n0\r\n";
        assert_eq!(avatar::read_skn(control).unwrap().texture, "x");
        let blank = b"audit-mesh\r\n\r\n1\r\nROOT\r\n0\r\n0\r\n0\r\n0\r\n0\r\n";
        assert!(
            avatar::read_skn(blank)
                .unwrap_err()
                .to_string()
                .contains("invalid avatar integer")
        );
    }
}
