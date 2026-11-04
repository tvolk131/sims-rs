//! Resource linking and an explicit two-object laboratory fixture for VM development.
use crate::Index;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use sims_formats::{
    behavior::Behavior,
    execution::{self, Slots},
    iff::Iff,
};
use sims_vm::{Definition, Module, Object, Program, Routine, World};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Fixed Global STR# cache populated by original59d860, in native cache order.
pub const NATIVE_ANIMATION_TABLES: [u16; 13] = [
    150, 151, 152, 153, 154, 155, 130, 128, 156, 157, 158, 159, 160,
];
/// Tables omitted by the earlier loader's fixed whitelist. Research snapshots
/// without a retained string catalog may reconstruct only this exact older
/// subset when their full original program fingerprint agrees.
pub const PREVIOUSLY_UNLINKED_ANIMATION_TABLES: [u16; 5] = [152, 153, 158, 159, 160];

/// Explicit immutable resource preparation, not an animation/VM update. Old
/// managed catalogs are validated before entering this upgrade. Stage all
/// decoded tables before publication; genuine absent resources stay absent.
pub fn prepare_native_animation_tables(index: &Index, program: &mut Program) -> Result<usize> {
    let module = program
        .modules
        .get(&program.globals)
        .context("animation globals absent")?;
    let iff = Iff::parse(index.resources[index.find(&program.globals)?].read()?)?;
    anyhow::ensure!(
        format!("{:x}", Sha256::digest(&iff.bytes)) == module.content_hash,
        "native animation global resource changed"
    );
    let mut prepared = BTreeMap::new();
    for id in NATIVE_ANIMATION_TABLES {
        if module.strings.contains_key(&id) {
            continue;
        }
        if let Some(chunk) = iff.get(b"STR#", id) {
            let table = sims_formats::strings::StringTable::parse(iff.data(chunk))
                .with_context(|| format!("native animation STR# {id}"))?;
            prepared.insert(
                id,
                table
                    .entries
                    .into_iter()
                    .filter(|e| e.language == 1)
                    .map(|e| e.text)
                    .collect(),
            );
        }
    }
    let count = prepared.len();
    program
        .modules
        .get_mut(&program.globals)
        .unwrap()
        .strings
        .extend(prepared);
    Ok(count)
}

/// Reconstruct exactly the saved definition profile before validating a
/// retained program fingerprint. Absence in `normalized` means the historical
/// raw-word linker, not permission to upgrade the checkpoint. Fresh dynamic
/// links continue to use native431759 normalization through `load_program`.
/// Other parsed fields read complete words inside the original minimum OBJD
/// extent and are unchanged by431759's version/tail normalization.
pub fn restore_definition_profiles(
    index: &Index,
    program: &mut Program,
    normalized: &BTreeSet<String>,
) -> Result<()> {
    anyhow::ensure!(
        normalized
            .iter()
            .all(|name| program.modules.contains_key(name)),
        "saved normalized definition module absent"
    );
    let mut restored = Vec::new();
    for (name, module) in &program.modules {
        let iff = Iff::parse(index.resources[index.find(name)?].read()?)?;
        anyhow::ensure!(
            format!("{:x}", Sha256::digest(&iff.bytes)) == module.content_hash,
            "definition profile resource changed: {name}"
        );
        let native = normalized.contains(name);
        let mut words = BTreeMap::new();
        for chunk in iff.chunks.iter().filter(|chunk| chunk.tag == *b"OBJD") {
            let mut data = iff.data(chunk).to_vec();
            if native {
                sims_sim::object_catalog::definition::normalize(&mut data)
                    .map_err(anyhow::Error::msg)?;
            }
            words.insert(
                chunk.id,
                data.chunks_exact(2)
                    .take(108)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect(),
            );
        }
        restored.push((name.clone(), native, words));
    }
    // Read/validate every immutable source before publishing any profile.
    for (name, native, words) in restored {
        let module = program.modules.get_mut(&name).unwrap();
        module.object_definition_words = words;
        module.normalized_object_definitions = native;
    }
    Ok(())
}

pub fn load_program(index: &Index, private: &str, global: &str) -> Result<Program> {
    let local = index.find(private)?;
    let global = index.find(global)?;
    let mut program = Program {
        // This loader selects English STR# entries below.
        text_language: Some(1),
        globals: index.resources[global].locator.clone(),
        modules: BTreeMap::new(),
        ..Program::default()
    };
    let mut pending = VecDeque::from([local, global]);
    // Some shipped files retain obsolete TTABs which no OBJD selects (e.g.
    // Stoves #128). Preserve strict parsing for every selected interaction table;
    // an unrelated historical chunk must not prevent loading the usable objects.
    let mut legacy_tables = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop_front() {
        let res = &index.resources[id];
        if program.modules.contains_key(&res.locator) {
            continue;
        }
        if program.modules.len() >= 32 {
            bail!("GLOB dependency graph exceeds 32 source modules");
        }
        let iff = Iff::parse(res.read()?).with_context(|| res.locator.clone())?;
        let mut module = Module {
            source: res.locator.clone(),
            content_hash: format!("{:x}", Sha256::digest(&iff.bytes)),
            normalized_object_definitions: true,
            ..Module::default()
        };
        let mut function_tables = BTreeMap::new();
        for chunk in &iff.chunks {
            let bytes = iff.data(chunk);
            let duplicate = match &chunk.tag {
                b"TTAB" => {
                    if bytes.len() >= 4
                        && bytes[..2] != [0, 0]
                        && u16::from_le_bytes([bytes[2], bytes[3]]) < 4
                    {
                        legacy_tables.insert((res.locator.clone(), chunk.id));
                        continue;
                    }
                    let table = sims_formats::behavior::InteractionTable::parse(bytes)?;
                    let labels = iff
                        .get(b"TTAs", chunk.id)
                        .map(|c| sims_formats::strings::StringTable::parse(iff.data(c)))
                        .transpose()?;
                    let entries = table
                        .interactions
                        .iter()
                        .map(|i| sims_vm::program::MenuEntry {
                            id: i.string_index,
                            flags: i.flags,
                            action: i.action,
                            check: i.test,
                            autonomy: i.into(),
                            label: labels
                                .as_ref()
                                .and_then(|t| t.get(i.string_index as usize, 1))
                                .map(str::to_owned)
                                .unwrap_or_else(|| format!("Interaction {}", i.string_index)),
                        })
                        .collect();
                    module
                        .interaction_tables
                        .insert(chunk.id, entries)
                        .is_some()
                }
                b"OBJf" => function_tables
                    .insert(chunk.id, execution::ObjectFunctions::parse(bytes)?)
                    .is_some(),
                b"OBJD" => {
                    module.object_definition_order.push(chunk.id);
                    // Native43ba52 calls431759 before publishing a registered
                    // definition. Keep the original IFF/hash immutable while
                    // interpreting the resident216-byte definition owner.
                    let mut resident = bytes.to_vec();
                    sims_sim::object_catalog::definition::normalize(&mut resident)
                        .map_err(anyhow::Error::msg)?;
                    let def = sims_formats::behavior::ObjectDefinition::parse(&resident)?;
                    module.object_definition_words.insert(
                        chunk.id,
                        resident
                            .chunks_exact(2)
                            .take(108)
                            .map(|b| i16::from_le_bytes([b[0], b[1]]))
                            .collect(),
                    );
                    module
                        .object_stand_functions
                        .insert(chunk.id, def.stand_function);
                    module
                        .object_animation_tables
                        .insert(chunk.id, def.animation_table)
                        .is_some()
                }
                b"STR#"
                    if [
                        128, 129, 130, 131, 132, 150, 151, 154, 155, 156, 157, 301, 303, 304, 305,
                    ]
                    .contains(&chunk.id)
                        || (module.source == program.globals
                            && (NATIVE_ANIMATION_TABLES.contains(&chunk.id)
                                || (501..=504).contains(&chunk.id)))
                        || (res.name.eq_ignore_ascii_case("UIText.iff") && chunk.id == 142) =>
                {
                    let table = sims_formats::strings::StringTable::parse(bytes)?;
                    if chunk.id == 301 {
                        module.dialog_marked = table
                            .entries
                            .iter()
                            .filter(|e| e.language == 1)
                            .enumerate()
                            .filter_map(|(i, e)| e.comment.starts_with('!').then_some(i + 1))
                            .collect();
                    }
                    let entries = table
                        .entries
                        .iter()
                        .filter(|e| e.language == 1)
                        .map(|e| e.text.clone())
                        .collect();
                    module.strings.insert(chunk.id, entries).is_some()
                }
                b"FWAV" => {
                    let end = bytes
                        .iter()
                        .position(|b| *b == 0)
                        .context("unterminated FWAV name")?;
                    if end == 0 || end > 1024 || !bytes[..end].is_ascii() {
                        bail!("unsupported FWAV event name");
                    }
                    module
                        .sounds
                        .insert(chunk.id, String::from_utf8(bytes[..end].to_vec())?)
                        .is_some()
                }
                b"BHAV" => {
                    let body = Behavior::parse(bytes)?;
                    module.routine_versions.insert(
                        chunk.id,
                        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                    );
                    module
                        .routines
                        .insert(
                            chunk.id,
                            Routine {
                                name: chunk.label.clone(),
                                body,
                            },
                        )
                        .is_some()
                }
                b"BCON" => module
                    .constants
                    .insert(chunk.id, execution::constants(bytes)?)
                    .is_some(),
                b"FCNS" => module
                    .named_constants
                    .insert(chunk.id, sims_formats::tuning::Constants::parse(bytes)?)
                    .is_some(),
                b"SLOT" => module
                    .slots
                    .insert(chunk.id, Slots::parse(bytes)?)
                    .is_some(),
                b"GLOB" => {
                    if module.semiglobal.is_some() {
                        bail!("multiple GLOB links in {}", module.source);
                    }
                    let name = execution::global_link(bytes)?;
                    let filename = if name.to_ascii_lowercase().ends_with(".iff") {
                        name
                    } else {
                        format!("{name}.iff")
                    };
                    let next = index
                        .find(&filename)
                        .with_context(|| format!("GLOB dependency from {}", module.source))?;
                    module.semiglobal = Some(index.resources[next].locator.clone());
                    pending.push_back(next);
                    false
                }
                _ => false,
            };
            if duplicate {
                bail!(
                    "ambiguous {} #{} in {}",
                    chunk.kind(),
                    chunk.id,
                    module.source
                );
            }
        }
        // Non-default animation table IDs come from OBJD, rather than a filename convention.
        for chunk in iff.chunks.iter().filter(|c| {
            c.tag == *b"STR#"
                && ![
                    128, 129, 130, 131, 132, 150, 151, 154, 155, 156, 157, 301, 303, 304, 305,
                ]
                .contains(&c.id)
                && !(module.source == program.globals && NATIVE_ANIMATION_TABLES.contains(&c.id))
        }) {
            if module
                .object_animation_tables
                .values()
                .any(|id| *id == chunk.id)
            {
                let table = sims_formats::strings::StringTable::parse(iff.data(chunk))?;
                let entries = table
                    .entries
                    .iter()
                    .filter(|e| e.language == 1)
                    .map(|e| e.text.clone())
                    .collect();
                if module.strings.insert(chunk.id, entries).is_some() {
                    bail!("ambiguous animation STR# {}", chunk.id);
                }
            }
        }
        for chunk in iff.chunks.iter().filter(|c| c.tag == *b"OBJD") {
            let resident: Vec<u8> = module.object_definition_words[&chunk.id]
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect();
            let _def = sims_formats::behavior::ObjectDefinition::parse(&resident)?;
            // Native 0x4061e3 prefers the legacy CATS resource. Its separate
            // encoding/fallback path remains a display-name boundary.
            if iff.get(b"CATS", _def.catalog).is_none()
                && let Some(text) = iff.get(b"CTSS", _def.catalog)
            {
                let names = sims_formats::strings::StringTable::parse(iff.data(text))?;
                if let Some(name) = names.get(0, 1).filter(|s| !s.is_empty()) {
                    module.catalog_names.insert(chunk.id, name.into());
                }
            }
            let (entries, source) = if let Some(table) = function_tables.get(&chunk.id) {
                (table.entries.clone(), "OBJf")
            } else {
                let offsets = [
                    0x56, 0x0a, 0x5e, 0x68, 0x78, 0x2a, 0x7c, 0x64, 0x9c, 0x58, 0x94, 0x60, 0x5a,
                    0x6a, 0x6e, 0x22, 0x0c, 0x18, 0x30, 0x32, 0x34, 0x36, 0x38, 0x3a, 0x3c, 0x3e,
                    0x40, 0x42, 0x76, 0xa0,
                ];
                let bytes = &resident;
                (
                    offsets
                        .iter()
                        .map(|&offset| execution::ObjectFunction {
                            condition: 0,
                            action: bytes
                                .get(offset..offset + 2)
                                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                                .unwrap_or(0),
                        })
                        .collect(),
                    "OBJD fallback",
                )
            };
            module
                .object_function_sources
                .insert(chunk.id, source.into());
            module.object_functions.insert(chunk.id, entries);
        }
        if module
            .routines
            .values()
            .any(|r| r.body.instructions.iter().any(|i| i.opcode == 0x24))
        {
            pending.push_back(index.find("UIText.iff")?);
        }
        program.modules.insert(module.source.clone(), module);
    }
    load_frame_strings(index, &mut program)?;
    // Native cached TTAB lookup tries the private resource, then its linked
    // semiglobal. Person interaction 33 lives in PersonGlobals, not TemplatePerson.
    // SAnimator reads the body StringSet selected by OBJD +0x26, then the
    // semiglobal skin-tone catalog. Load those exact dependencies instead of
    // parsing every (occasionally obsolete) STR# chunk in the character file.
    let mut person_strings = std::collections::BTreeSet::new();
    let mut body_profiles = std::collections::BTreeSet::new();
    for (source, module) in &program.modules {
        for words in module
            .object_definition_words
            .values()
            .filter(|w| w[9] == 2)
        {
            person_strings.insert((source.clone(), words[19] as u16));
            body_profiles.insert((source.clone(), words[19] as u16));
            if words[51] != 0 {
                person_strings.insert((source.clone(), words[51] as u16));
            }
            if let Some(shared) = &module.semiglobal {
                person_strings.insert((shared.clone(), 8192));
            }
        }
    }
    for (source, id) in person_strings {
        let module = program
            .modules
            .get_mut(&source)
            .context("person StringSet module absent")?;
        if module.strings.contains_key(&id) {
            continue;
        }
        let iff = Iff::parse(index.resources[index.find(&source)?].read()?)?;
        if let Some(chunk) = iff.get(b"STR#", id) {
            let table = sims_formats::strings::StringTable::parse(iff.data(chunk))?;
            module.strings.insert(
                id,
                table
                    .entries
                    .into_iter()
                    .filter(|e| e.language == 1)
                    .map(|e| e.text)
                    .collect(),
            );
        }
    }
    // 457f60 constructs BodyStrings through 587b4b before animator use. Only
    // OBJD +0x26 body profiles receive this rule, not animation or skin-tone
    // StringSets. Retain the source hash and the original file unchanged.
    for (source, id) in body_profiles {
        if let Some(strings) = program
            .modules
            .get_mut(&source)
            .unwrap()
            .strings
            .get_mut(&id)
        {
            crate::body_profile::normalize(strings);
        }
    }
    let mut inherited = vec![];
    for (source, module) in &program.modules {
        for (&object, words) in &module.object_definition_words {
            if module.object_interactions.contains_key(&object) {
                continue;
            }
            let mut table = words.get(7).copied().unwrap_or(0);
            if table == -1 {
                // Native 0x45726a -> 0x45728d -> 0x43ae0a: the catalog
                // definition in this resource file supplies a part's TTAB ID.
                table = module
                    .object_definition_order
                    .iter()
                    .find_map(|id| {
                        let parent = &module.object_definition_words[id];
                        (words[10] != 0 && parent[10] == words[10] && parent[11] == -1)
                            .then_some(parent[7])
                    })
                    .unwrap_or(0);
            }
            if table <= 0 {
                inherited.push((source.clone(), object, Vec::new()));
                continue;
            }
            if legacy_tables.contains(&(source.clone(), table as u16))
                || (!module.interaction_tables.contains_key(&(table as u16))
                    && module
                        .semiglobal
                        .as_ref()
                        .is_some_and(|s| legacy_tables.contains(&(s.clone(), table as u16))))
            {
                bail!("selected legacy TTAB #{table} for {source} OBJD #{object} is unsupported");
            }
            let entries = module.interaction_tables.get(&(table as u16)).or_else(|| {
                module
                    .semiglobal
                    .as_ref()
                    .and_then(|s| program.modules.get(s))
                    .and_then(|m| m.interaction_tables.get(&(table as u16)))
            });
            if let Some(entries) = entries {
                inherited.push((source.clone(), object, entries.clone()));
            }
        }
    }
    for (source, object, entries) in inherited {
        program
            .modules
            .get_mut(&source)
            .unwrap()
            .object_interactions
            .insert(object, entries);
    }
    Ok(program)
}

/// Reconstruct the exact immutable StringSet dependency set of a managed save.
/// Native person loading can add a table to an already linked shared module;
/// loading that module by filename alone does not reproduce the dependency.
pub fn restore_string_dependencies(
    index: &Index,
    program: &mut Program,
    tables: &BTreeMap<String, BTreeSet<u16>>,
) -> Result<()> {
    anyhow::ensure!(
        program.modules.keys().eq(tables.keys()),
        "saved string module set differs"
    );
    for (source, ids) in tables {
        let module = program.modules.get_mut(source).unwrap();
        let iff = Iff::parse(index.resources[index.find(source)?].read()?)?;
        let mut strings = BTreeMap::new();
        for id in ids {
            let chunk = iff
                .get(b"STR#", *id)
                .with_context(|| format!("missing saved STR# {source}:{id}"))?;
            let table = sims_formats::strings::StringTable::parse(iff.data(chunk))?;
            let mut entries: Vec<_> = table
                .entries
                .into_iter()
                .filter(|e| e.language == 1)
                .map(|e| e.text)
                .collect();
            if module
                .object_definition_words
                .values()
                .any(|w| w[9] == 2 && w[19] as u16 == *id)
            {
                crate::body_profile::normalize(&mut entries);
            }
            strings.insert(*id, entries);
        }
        module.strings = strings;
    }
    Ok(())
}

/// Shared behaviors keep the caller frame's private resource context. Native
/// Change Action String 0x44cc07 -> 0x416920 selects that context's file, even
/// when the instruction itself lives in a semiglobal/global BHAV. Discover
/// dependencies after linking GLOBs so tables referenced only there are loaded.
fn load_frame_strings(index: &Index, program: &mut Program) -> Result<()> {
    let mut needed: BTreeMap<String, BTreeSet<u16>> = BTreeMap::new();
    for (owner, module) in &program.modules {
        for source in [
            Some(owner),
            Some(&program.globals),
            module.semiglobal.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            for instruction in program.modules[source]
                .routines
                .values()
                .flat_map(|r| &r.body.instructions)
            {
                if instruction.opcode != 50
                    && !(instruction.opcode == 28 && instruction.operands[2] & 1 == 0)
                {
                    continue;
                }
                let table = u16::from_le_bytes([instruction.operands[0], instruction.operands[1]]);
                if !module.strings.contains_key(&table) {
                    needed.entry(owner.clone()).or_default().insert(table);
                }
            }
        }
    }
    for (source, tables) in needed {
        let iff = Iff::parse(index.resources[index.find(&source)?].read()?)?;
        let module = program.modules.get_mut(&source).unwrap();
        for id in tables {
            if let Some(chunk) = iff.get(b"STR#", id) {
                let table = sims_formats::strings::StringTable::parse(iff.data(chunk))
                    .with_context(|| format!("frame string dependency {source} STR# {id}"))?;
                module.strings.insert(
                    id,
                    table
                        .entries
                        .into_iter()
                        .filter(|e| e.language == 1)
                        .map(|e| e.text)
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

/// Load explicit animation-only BCF catalogs. Name resolution uses embedded clip names,
/// not guessed filenames. Original CFP skill links are resolved with normal ambiguity checks.
pub fn load_animation_catalogs(
    index: &Index,
    program: &mut Program,
    catalogs: &[&str],
) -> Result<()> {
    use sims_formats::animation::{read_bcf, read_cfp};
    use sims_vm::program::AnimationAsset;
    let mut assets = BTreeMap::new();
    for selector in catalogs {
        let resource = &index.resources[index.find(selector)?];
        let bytes = resource.read()?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        for clip in read_bcf(&bytes).with_context(|| resource.locator.clone())? {
            let cfp_name = format!("{}.cfp", clip.skill);
            let cfp_id = if let Some((prefix, _)) = resource.locator.rsplit_once('\\') {
                index
                    .find(&format!("{prefix}\\{cfp_name}"))
                    .or_else(|_| index.find(&cfp_name))?
            } else {
                index.find(&cfp_name)?
            };
            let cfp = &index.resources[cfp_id];
            let data = cfp.read()?;
            let samples = read_cfp(&data, clip.translation_count, clip.rotation_count)
                .with_context(|| cfp.locator.clone())?;
            let name = clip.name.to_ascii_lowercase();
            if program.animations.contains_key(&name) || assets.contains_key(&name) {
                bail!("ambiguous animation name {name}");
            }
            assets.insert(
                name,
                AnimationAsset {
                    source: resource.locator.clone(),
                    content_hash: hash.clone(),
                    samples_source: cfp.locator.clone(),
                    samples_hash: format!("{:x}", Sha256::digest(&data)),
                    clip,
                    samples,
                },
            );
        }
    }
    program.animations.extend(assets);
    Ok(())
}

/// Shared original sprite-headline resource. Ambiguous resource names fail;
/// the resource identity and content hash are retained in the program catalog.
pub fn load_headline_catalog(index: &Index, program: &mut Program) -> Result<()> {
    let resource = &index.resources[index.find("Sprites.iff")?];
    let iff = Iff::parse(resource.read()?)?;
    let mut catalog = sims_vm::headline::Catalog {
        source: resource.locator.clone(),
        content_hash: format!("{:x}", Sha256::digest(&iff.bytes)),
        ..Default::default()
    };
    for chunk in iff.all(b"SPR#") {
        let sprites = sims_formats::sprite::LegacySpriteSet::parse(iff.data(chunk))?;
        if catalog
            .frames
            .insert(chunk.id, sprites.frames.len() as u32)
            .is_some()
        {
            bail!("duplicate legacy sprite ID {}", chunk.id);
        }
    }
    program
        .modules
        .get_mut(&program.globals)
        .context("missing global module")?
        .headline_catalog = Some(catalog);
    Ok(())
}

/// Select the verified adult locomotion/idle profile; linked object and global
/// animation tables are resolved after household modules have been assembled.
pub fn load_household_animations(index: &Index, program: &mut Program) -> Result<()> {
    load_chair_animations(index, program)
}

pub fn load_chair_animations(index: &Index, program: &mut Program) -> Result<()> {
    load_headline_catalog(index, program)?;
    // The room's adult is explicitly based on the original template. This also
    // supplies definition-word queries performed by the chair's Main script.
    let person = load_program(index, r"People\TemplatePerson.iff", "Global.iff")?;
    for (source, module) in person.modules {
        program.modules.entry(source).or_insert(module);
    }
    load_animation_catalogs(
        index,
        program,
        &[
            "adult-heyyou.cmx.bcf",
            "adult-uncomfortable2.cmx.bcf",
            "ross-standing-loop.cmx.bcf",
            "adult-chair-sit-new.cmx.bcf",
            "adult-chair-stand-new.cmx.bcf",
            "a2o-sit-loop.cmx.bcf",
            "a2o-idle-sit-1.cmx.bcf",
            "a2o-idle-sit-2.cmx.bcf",
            "a2o-idle-sit-3.cmx.bcf",
        ],
    )?;
    load_locomotion_animations(index, program)?;
    load_animation_catalogs(
        index,
        program,
        &[
            r"ExpansionShared/ExpansionShared.far::Animations\a2o-idle-neutral-lstart.cmx.bcf",
            r"ExpansionShared/ExpansionShared.far::Animations\a2o-idle-neutral-mstart.cmx.bcf",
            r"ExpansionShared/ExpansionShared.far::Animations\a2o-idle-neutral-rstart.cmx.bcf",
            r"ExpansionShared/ExpansionShared.far::Animations\a2o-idle-neutral-rhips.cmx.bcf",
            r"ExpansionShared/ExpansionShared.far::Animations\a2o-idle-neutral-trans-lm.cmx.bcf",
            r"ExpansionShared/ExpansionShared.far::Animations\a2o-idle-neutral-trans-lr.cmx.bcf",
            r"ExpansionShared/ExpansionShared.far::Animations\a2o-idle-neutral-trans-mr.cmx.bcf",
        ],
    )?;
    // Original person/failure trees can select more clips than the chair's
    // immediate Sit/Stand sequence. Resolve their tables through the same
    // systematic loader used by households, including route-failure feedback.
    crate::assets::load_linked_animations(index, program)?;
    Ok(())
}

/// Verified adult locomotion profile. Kept in the same content fingerprint as Sit.
pub fn load_locomotion_animations(index: &Index, program: &mut Program) -> Result<()> {
    let mut catalogs = vec![
        "ross-walking-loop.cmx.bcf".to_owned(),
        "adult-walking-stop.cmx.bcf".into(),
        "adult-stand.cmx.bcf".into(),
        "adult-standing-turn-0.cmx.bcf".into(),
    ];
    for angle in [45, 90, 180] {
        for side in ["cw", "ccw"] {
            catalogs.push(format!("adult-standing-turn-{angle}-{side}.cmx.bcf"));
            if angle != 180 {
                for foot in ["left", "right"] {
                    catalogs.push(format!("adult-walking-turn-{angle}-{side}-{foot}.cmx.bcf"));
                }
            }
        }
    }
    load_animation_catalogs(
        index,
        program,
        &catalogs.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChairScenario {
    Empty,
    Occupied,
    AlreadySeated,
    AutonomousSeated,
}
impl std::str::FromStr for ChairScenario {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "empty" => Ok(Self::Empty),
            "occupied" => Ok(Self::Occupied),
            "already-seated" => Ok(Self::AlreadySeated),
            "autonomous-seated" => Ok(Self::AutonomousSeated),
            _ => {
                Err("scenario must be empty, occupied, already-seated, or autonomous-seated".into())
            }
        }
    }
}
pub struct ChairFixture {
    pub owner: String,
    pub title: String,
    pub action: u16,
    pub check: u16,
    pub args: Vec<i16>,
    pub world: World,
}
impl ChairFixture {
    pub fn load(
        index: &Index,
        resource: &str,
        object: u16,
        scenario: ChairScenario,
    ) -> Result<Self> {
        let doc = index.document(index.find(resource)?)?;
        let chair = doc
            .objects
            .iter()
            .find(|o| o.id == object)
            .context("chair OBJD not found")?;
        let table = doc
            .tables
            .iter()
            .find(|t| t.id == chair.value.table)
            .context("chair TTAB not found")?;
        let labels = doc.strings(b"TTAs", table.id)?;
        let interaction = table
            .value
            .interactions
            .iter()
            .find(|i| labels.get(i.string_index as usize, 1) == Some("Sit"))
            .context("Sit interaction not found")?;
        let slots = doc
            .iff
            .get(b"SLOT", chair.value.slots)
            .context("chair SLOT not found")?;
        let slots = Slots::parse(doc.iff.data(slots))?;
        let containment = slots.entries.iter().filter(|s| s.kind == 0).count();
        if containment == 0 || containment > 256 {
            bail!("fixture needs a real, bounded containment slot");
        }
        let person_doc = index.document(index.find(r"People\TemplatePerson.iff")?)?;
        let person_def = person_doc
            .objects
            .iter()
            .find(|o| o.id == 128)
            .context("adult template definition missing")?;
        let mut motives = [0f32.to_bits(); 16];
        motives[6] = (-50f32).to_bits(); // Explicit demo starting need, not a native default.
        let mut world = World::default();
        let mut actor = Object {
            definition: Some(Definition {
                source: person_doc.source.clone(),
                object: 128,
                guid: person_def.value.guid,
                slots: person_def.value.slots,
            }),
            motives: Some(motives),
            person: Some(BTreeMap::from([
                (0, 0),
                (37, 1),
                (42, 0),
                (58, 30),
                (64, 0),
                (65, 0),
                (62, 0),
                (73, 0),
            ])),
            data: BTreeMap::from([(1, 0), (17, 0), (52, 0), (54, 0)]),
            slots: vec![None],
            ..Object::default()
        };
        let p = load_program(index, resource, "Global.iff")?;
        let mut chair_object = Object::construct(
            Definition {
                source: doc.source.clone(),
                object,
                guid: chair.value.guid,
                slots: chair.value.slots,
            },
            &p,
            containment,
        )
        .map_err(anyhow::Error::msg)?;
        chair_object.data.insert(1, 0);
        chair_object.data.insert(11, 2);
        match scenario {
            ChairScenario::AlreadySeated => {
                chair_object.slots[0] = Some(1);
                actor.container = Some((2, 0));
                actor.person.as_mut().unwrap().insert(0, 1);
            }
            ChairScenario::Occupied => {
                chair_object.slots[0] = Some(3);
                world.objects.insert(
                    3,
                    Object {
                        data: BTreeMap::from([(1, 0)]),
                        container: Some((2, 0)),
                        person: Some(BTreeMap::from([(0, 1)])),
                        ..Object::default()
                    },
                );
            }
            ChairScenario::AutonomousSeated => {
                actor.person.as_mut().unwrap().insert(0, 1);
            }
            ChairScenario::Empty => (),
        }
        world.objects.insert(1, actor);
        world.objects.insert(2, chair_object);
        Ok(Self {
            owner: doc.source.clone(),
            title: doc.object_title(chair),
            action: interaction.action,
            check: interaction.test,
            args: vec![i16::from(scenario == ChairScenario::AutonomousSeated)],
            world,
        })
    }
}

/// An authored laboratory room, not an original lot importer. Geometry and collision
/// sizes are deliberately explicit; behavior and interaction slots come from the assets.
pub fn chair_room(
    program: &Program,
    fixture: ChairFixture,
    blocked: bool,
    facing: u8,
) -> Result<sims_sim::Simulation> {
    build_chair_room(program, fixture, blocked, facing, true)
}
/// User-directed fixture: two copies of the same original chair, one adult, no initial action.
pub fn two_chair_room(
    program: &Program,
    fixture: ChairFixture,
    facing: u8,
) -> Result<sims_sim::Simulation> {
    build_two_chair_room(program, fixture, facing, false)
}
/// Household checkpoint with original person Init on explicitly supplied storage.
/// Native person construction remains a separate compatibility boundary.
pub fn initialized_two_chair_room(
    program: &Program,
    fixture: ChairFixture,
    facing: u8,
) -> Result<sims_sim::Simulation> {
    build_two_chair_room(program, fixture, facing, true)
}
fn build_two_chair_room(
    program: &Program,
    fixture: ChairFixture,
    facing: u8,
    person_init: bool,
) -> Result<sims_sim::Simulation> {
    let mut sim = build_chair_room(program, fixture, false, facing, false)?;
    let mut chair = sim.vm.world.objects[&2].clone();
    let facing = facing.rotate_left(2);
    chair.data.insert(1, facing.trailing_zeros() as i16);
    chair.data.insert(11, 3);
    sim.vm.world.objects.insert(3, chair);
    let mut body = sim.bodies[&2].clone();
    body.position = sims_sim::Point::new(88, 88);
    body.facing = facing;
    sim.bodies.insert(3, body);
    sim.interactions
        .insert(1, sims_sim::interaction::ActorQueue::default());
    if person_init {
        let old = &sim.vm.world.objects[&1];
        if old.container.is_some() {
            bail!("initialized person fixture requires an uncontained adult");
        }
        let mut actor = sims_vm::Object::construct_person(
            old.definition.clone().unwrap(),
            program,
            sims_vm::person::BUILTIN_SLOT_HEIGHTS.len(),
        )
        .map_err(anyhow::Error::msg)?;
        actor
            .data
            .insert(67, sims_vm::person::BUILTIN_SLOT_HEIGHTS.len() as i16);
        actor.headline = Some(Default::default());
        // The authored resident profile supplies age after storage construction.
        actor
            .person
            .as_mut()
            .unwrap()
            .insert(58, old.person.as_ref().unwrap()[&58]);
        sim.vm.world.objects.insert(1, actor);
    }
    // Identity, placement and neighborhood membership remain authored inputs.
    // The initialized variant uses native person storage plus original Init.
    let actor = sim.vm.world.objects.get_mut(&1).unwrap();
    for index in 0..72 {
        actor.data.entry(index).or_insert(0);
    }
    for index in 0..256 {
        actor.person.as_mut().unwrap().entry(index).or_insert(0);
    }
    actor.data.insert(11, 1);
    actor.data.insert(29, 1);
    sim.vm.world.room_outdoors.insert(1, false);
    sim.vm
        .world
        .room_half_tiles
        .insert(1, sim.room.width * sim.room.height / 128);
    actor.attributes = vec![0; 8];
    actor.person.as_mut().unwrap().insert(31, 1);
    actor.person.as_mut().unwrap().insert(61, 1);
    sim.vm.world.person = Some(sims_vm::person::WorldState {
        neighbors: BTreeMap::from([(
            1,
            sims_vm::person::Neighbor {
                guid: actor.definition.as_ref().unwrap().guid,
                person: actor.person.clone().unwrap(),
                ..Default::default()
            },
        )]),
        controlled_family: 1,
        families: BTreeMap::from([(1, vec![1])]),
        family_homes: BTreeMap::from([(1, 1)]),
        lot_zoning: BTreeMap::from([(1, 1)]),
        object_order: vec![1, 2, 3],
        person_order: vec![1],
        ..Default::default()
    });
    sim.vm.world.globals.extend([(10, 1), (20, 0)]);
    sim.vm.world.actions = Some(sims_vm::action::WorldState {
        live_family: true,
        actors: BTreeMap::from([(1, sims_vm::action::Actor::default())]),
        ..Default::default()
    });
    // An explicitly supplied one-floor laboratory room. Refresh records real
    // invalidations; room scoring consumers remain outside this authored fixture.
    sim.vm.world.refresh = Some(sims_vm::refresh::RefreshState {
        visible_level: 1,
        draw: sim
            .vm
            .world
            .objects
            .keys()
            .map(|&id| (id, sims_vm::refresh::DrawState::constructed(1)))
            .collect(),
        rooms: BTreeMap::from([(1, sims_vm::refresh::RoomState::default())]),
        ..Default::default()
    });
    for id in [2, 3] {
        sim.vm
            .world
            .objects
            .get_mut(&id)
            .unwrap()
            .data
            .insert(29, 1);
        sim.vm
            .reset_object_lifecycle(program, id, sims_vm::construction::ObjectDate::default())
            .map_err(anyhow::Error::msg)?;
    }
    let main = if person_init {
        sim.vm.start_object_lifecycle(program, 1)
    } else {
        sim.vm.start_supplied_person_main(program, 1)
    }
    .map_err(anyhow::Error::msg)?;
    sim.interactions.get_mut(&1).unwrap().main = Some(main);
    // Finish synchronous Init before exposing menus or accepting commands.
    loop {
        let step = sim
            .run_fixture_tick(program, 1000)
            .map_err(anyhow::Error::msg)?;
        match step.stop {
            sims_sim::StepStop::Budget => continue,
            sims_sim::StepStop::Faulted => bail!(
                "chair lifecycle initialization failed: {:?}",
                sim.vm.threads
            ),
            _ => break,
        }
    }
    sims_sim::Snapshot::capture(program, &sim)
        .restore(program)
        .map_err(anyhow::Error::msg)?;
    Ok(sim)
}
fn build_chair_room(
    program: &Program,
    fixture: ChairFixture,
    blocked: bool,
    facing: u8,
    start: bool,
) -> Result<sims_sim::Simulation> {
    use sims_sim::{Action, Body, Point, Rect, Room, Simulation};
    if !matches!(facing, 1 | 4 | 16 | 64) {
        bail!("chair facing must be 1, 4, 16, or 64");
    }
    let mut world = fixture.world;
    world
        .objects
        .get_mut(&2)
        .unwrap()
        .data
        .insert(1, facing.trailing_zeros() as i16);
    let bodies = world
        .objects
        .iter()
        .map(|(&id, o)| {
            let contained = o.container.is_some();
            (
                id,
                Body {
                    level: 1,
                    position: if id == 2 || contained {
                        Point::new(88, 56)
                    } else {
                        Point::new(24, 56)
                    },
                    facing: 1u8 << (*o.data.get(&1).unwrap() as u32),
                    radius: if id == 2 { 6 } else { 4 },
                    elevation: if contained { 7 } else { 0 },
                },
            )
        })
        .collect();
    let room = Room {
        width: 128,
        height: 112,
        obstacles: vec![Rect {
            min: Point::new(48, if blocked { 0 } else { 32 }),
            max: Point::new(56, if blocked { 112 } else { 80 }),
        }],
    };
    let mut sim =
        Simulation::new(sims_vm::Vm::new(world, 1), room, bodies).map_err(anyhow::Error::msg)?;
    if start {
        sim.queue_interaction(
            program,
            fixture.check,
            Action {
                owner: fixture.owner,
                bhav: fixture.action,
                actor: 1,
                target: 2,
                args: fixture.args,
            },
        )
        .map_err(anyhow::Error::msg)?;
    }
    Ok(sim)
}

/// Explicit single-object laboratory storage, not a native lot/object constructor.
/// Used to inspect Init/Main execution without inventing architectural services.
pub fn lifecycle_fixture(index: &Index, resource: &str, object: u16) -> Result<(String, World)> {
    let doc = index.document(index.find(resource)?)?;
    let def = doc
        .objects
        .iter()
        .find(|o| o.id == object)
        .context("OBJD not found")?;
    let slots = doc
        .iff
        .get(b"SLOT", def.value.slots)
        .map(|c| Slots::parse(doc.iff.data(c)))
        .transpose()?
        .map_or(0, |s| s.entries.iter().filter(|s| s.kind == 0).count());
    let mut world = World::default();
    let p = load_program(index, resource, "Global.iff")?;
    let mut instance = Object::construct(
        Definition {
            source: doc.source.clone(),
            object,
            guid: def.value.guid,
            slots: def.value.slots,
        },
        &p,
        slots,
    )
    .map_err(anyhow::Error::msg)?;
    instance.data.insert(11, 1); // Registry assignment after construction.
    instance.data.insert(29, -5); // Explicit off-lot laboratory placement.
    world.objects.insert(1, instance);
    world.refresh = Some(sims_vm::refresh::RefreshState {
        draw: BTreeMap::from([(1, sims_vm::refresh::DrawState::constructed(0))]),
        ..Default::default()
    });
    Ok((doc.source, world))
}
