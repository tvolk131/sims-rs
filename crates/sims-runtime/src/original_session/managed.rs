//! Version2 player restoration: deserialize owned state and link immutable
//! behavior/animation assets, without running native starting-house activation.
//! Version1 remains available for preserved research checkpoints.
use super::*;
use sims_content::owned_resources::Neighborhood;
pub(in crate::original_session) mod completed_owned;

/// In-memory origin policy; restored from the explicit managed content schema.
/// This is not inferred from historical source hash differences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Policy {
    HistoricalSchema1,
    CompletedOwnedSchema2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    pub schema: u32,
    /// Schema1: House; schema2: completed-owned metadata envelope. Binary Serde preserves every float bit,
    /// including stale disabled-tracking NaNs which JSON cannot roundtrip.
    pub house: Vec<u8>,
    pub owned: Neighborhood,
    pub catalog: Catalog,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub globals: String,
    pub text_language: Option<u32>,
    pub neighbor_resource_flags: Option<BTreeMap<u32, u32>>,
    pub resource_storage: Option<Vec<sims_vm::resource_storage::Definition>>,
    pub accessories: BTreeMap<String, BTreeMap<String, String>>,
    /// Context-dependent immutable STR# dependencies discovered while linking
    /// private person/global resources. Module filenames alone lose this set.
    pub strings: BTreeMap<String, BTreeSet<u16>>,
    pub careers: bool,
}
impl Session {
    /// Restore a completed-owned checkpoint with its independently reconstructed
    /// exact resident Program. A completed capture's lazy cache is not the same
    /// as linking every routine in the backing files. The complete fingerprint,
    /// source/version manifests and catalog must all agree before restoration.
    pub fn restore_completed_owned_with_program(
        index: &Index,
        saved: Saved,
        program: Program,
    ) -> Result<Self> {
        ensure!(
            saved.version == 2 && saved.managed.as_ref().is_some_and(|m| m.schema == 2),
            "retained Program restore requires completed-owned schema2"
        );
        restore_inner(index, saved, Some(program))
    }
    /// Pure capture. Player applications choose safe save boundaries; no VM,
    /// callback, script, resource-write or RNG work is driven to manufacture one.
    pub fn save_managed(&self) -> Result<Saved> {
        if let Some(frontend) = &self.host.presentation.frontend_context {
            frontend.validate(self)?;
        }
        if let Some(inputs) = &self.host.presentation.retained_tool_inputs {
            inputs.validate(self)?;
        }
        if let Some(parent) = &self.host.presentation.application_parent {
            parent.validate(
                self.host.presentation.clock.as_ref(),
                self.simulation.vm.tick,
            )?;
        }
        self.validate_neighborhood_input()?;
        if let Some(properties) = &self.host.presentation.property_refresh {
            properties.validate(self)?;
        }
        if self.managed_policy == Policy::CompletedOwnedSchema2 {
            return completed_owned::save(self);
        }
        let owned = Neighborhood::capture(&self.index, &self.house.neighborhood_source)?;
        let hashes = owned.hashes();
        for (source, hash) in &self.house.sources {
            if let Some(current) = hashes.get(source) {
                ensure!(
                    current == hash,
                    "original import source changed before ownership capture: {source}"
                );
            }
        }
        let content = Content {
            schema: 1,
            house: postcard::to_allocvec(&self.house)
                .map_err(|e| anyhow::anyhow!("encode managed house: {e}"))?,
            owned,
            catalog: Catalog::capture(self),
        };
        let mut saved = self.save();
        saved.version = 2;
        saved.managed = Some(Box::new(content));
        Ok(saved)
    }
}
impl Catalog {
    fn capture(session: &Session) -> Self {
        Self::from_program(&session.program)
    }
    fn from_program(program: &Program) -> Self {
        Self {
            globals: program.globals.clone(),
            text_language: program.text_language,
            neighbor_resource_flags: program.neighbor_resource_flags.clone(),
            resource_storage: program.resource_storage.clone(),
            accessories: program.accessories.clone(),
            strings: program
                .modules
                .iter()
                .map(|(name, module)| (name.clone(), module.strings.keys().copied().collect()))
                .collect(),
            careers: program.careers.is_some(),
        }
    }
}

pub(super) fn restore(index: &Index, saved: Saved) -> Result<Session> {
    restore_inner(index, saved, None)
}
fn restore_inner(index: &Index, mut saved: Saved, retained: Option<Program>) -> Result<Session> {
    ensure!(saved.version == 2, "managed session version differs");
    ensure!(
        saved.simulation.state.native_scheduler.is_some()
            && saved.simulation.state.fixture_driver.is_none(),
        "managed household registry continuation absent"
    );
    let content = saved
        .managed
        .take()
        .context("version2 managed content absent")?;
    ensure!(
        matches!(content.schema, 1 | 2) && content.house.len() <= 128 * 1024 * 1024,
        "unsupported managed content schema/size"
    );
    let (house, completed) = if content.schema == 1 {
        let (house, remaining): (House, &[u8]) = postcard::take_from_bytes(&content.house)
            .map_err(|e| anyhow::anyhow!("decode managed house: {e}"))?;
        ensure!(remaining.is_empty(), "trailing managed house metadata");
        (house, None)
    } else {
        let metadata = completed_owned::decode(&content.house)?;
        (metadata.house, Some((metadata.modules, metadata.immutable)))
    };
    ensure!(
        house.source == saved.source
            && house.sources == saved.source_hashes
            && house.neighborhood_source == format!("{}/Neighborhood.iff", content.owned.directory),
        "managed import identity differs"
    );
    ensure!(
        house.width == house.height && (3..=64).contains(&house.width) && house.floors.len() == 2,
        "managed house geometry unavailable"
    );
    let mounted = if completed.is_some() {
        completed_owned::mount(index, &content.owned)?
    } else {
        content.owned.mount(index)?
    };
    let index = Arc::new(completed_baseline::bind_aliases(
        mounted,
        saved.view.resource_aliases.as_ref(),
    )?);
    // Parsed import/catalog metadata is retained state, not permission to
    // silently use a different immutable asset after moving the save.
    let mut hashes = if let Some((modules, immutable)) = &completed {
        completed_owned::validate(
            &index,
            &house,
            &content.catalog,
            modules,
            immutable,
            &saved.modules,
        )?;
        BTreeMap::new()
    } else {
        house.sources.clone()
    };
    for (source, hash) in content
        .catalog
        .accessories
        .values()
        .flat_map(|a| a.iter())
        .chain(
            content
                .catalog
                .resource_storage
                .iter()
                .flatten()
                .map(|d| (&d.source, &d.content_hash)),
        )
    {
        if completed.is_some() {
            continue;
        }
        if let Some(old) = hashes.insert(source.clone(), hash.clone()) {
            ensure!(old == *hash, "managed asset hashes conflict: {source}");
        }
    }
    use sha2::{Digest, Sha256};
    for (source, hash) in hashes {
        let bytes = index.resources[index.find(&source)?].read()?;
        ensure!(
            format!("{:x}", Sha256::digest(bytes)) == hash,
            "managed import/catalog asset changed: {source}"
        );
    }
    let mut program = if let Some(program) = retained {
        ensure!(
            completed.is_some(),
            "retained Program restore lost completed source ownership"
        );
        ensure!(
            postcard::to_allocvec(&Catalog::from_program(&program))?
                == postcard::to_allocvec(&content.catalog)?,
            "retained Program/catalog differs"
        );
        ensure!(
            program.modules.keys().cloned().collect::<BTreeSet<_>>() == saved.modules
                && program.animations.keys().cloned().collect::<BTreeSet<_>>() == saved.animations
                && program.native_animation_absences == saved.native_animation_absences
                && program
                    .modules
                    .iter()
                    .filter(|(_, m)| m.normalized_object_definitions)
                    .map(|(n, _)| n.clone())
                    .collect::<BTreeSet<_>>()
                    == saved.normalized_object_definitions
                && program
                    .modules
                    .get(&program.globals)
                    .is_some_and(|m| m.headline_catalog.is_some())
                    == saved.headline_catalog,
            "retained Program declared catalog differs"
        );
        program
    } else {
        link(
            &index,
            content.catalog,
            &saved.modules,
            &saved.animations,
            saved.headline_catalog,
            &saved.normalized_object_definitions,
        )?
    };
    super::restore_native_animation_absences(&index, &mut program, &saved.native_animation_absences)?;
    if let Some((modules, _)) = &completed {
        completed_owned::validate_modules(&program, modules)?;
        super::completed_baseline::resources(
            &index,
            &house.source,
            &house.neighborhood_source,
            &program,
        )?;
    }
    // Snapshot::restore checks the complete program fingerprint, VM requests,
    // registry/frame ownership, physical links and every retained simulation.
    let simulation = saved
        .simulation
        .restore(&program)
        .map_err(anyhow::Error::msg)?;
    let host = character_loading::live::Host::restore(&index, saved.view, saved.assets)?;
    let mut session = Session {
        index,
        managed_policy: if completed.is_some() {
            Policy::CompletedOwnedSchema2
        } else {
            Policy::HistoricalSchema1
        },
        house,
        program,
        simulation,
        host,
        options: saved.options,
    };
    session.validate_restored()?;
    // Preserve and validate the exact saved catalog first. Completing the
    // original59d860 immutable cache changes no retained VM/animator state;
    // later lookups must not mistake an unlinked swimming table for absence.
    if completed.is_none() {
        sims_content::vm::prepare_native_animation_tables(&session.index, &mut session.program)?;
    }
    Ok(session)
}

fn link(
    index: &Index,
    catalog: Catalog,
    modules: &BTreeSet<String>,
    animations: &BTreeSet<String>,
    headlines: bool,
    normalized_definitions: &BTreeSet<String>,
) -> Result<Program> {
    let mut program = Program {
        globals: catalog.globals,
        text_language: catalog.text_language,
        neighbor_resource_flags: catalog.neighbor_resource_flags,
        resource_storage: catalog.resource_storage,
        accessories: catalog.accessories,
        careers: if catalog.careers {
            Some(sims_content::careers::load(&index)?)
        } else {
            None
        },
        ..Default::default()
    };
    for source in modules {
        if program.modules.contains_key(source) {
            continue;
        }
        let loaded = sims_content::vm::load_program(&index, source, &program.globals)?;
        ensure!(
            loaded.globals == program.globals,
            "managed global asset differs"
        );
        for (name, module) in loaded.modules {
            if let Some(old) = program.modules.get(&name) {
                ensure!(
                    old.content_hash == module.content_hash,
                    "managed module conflict: {name}"
                );
            } else {
                program.modules.insert(name, module);
            }
        }
    }
    sims_content::vm::restore_definition_profiles(index, &mut program, normalized_definitions)?;
    if headlines {
        sims_content::vm::load_headline_catalog(&index, &mut program)?;
    }
    sims_content::vm::restore_string_dependencies(index, &mut program, &catalog.strings)?;
    let loaded =
        sims_content::assets::load_named_native_animations(&index, &mut program, animations)?;
    ensure!(
        loaded.unresolved.is_empty(),
        "managed animation assets missing: {:?}",
        loaded.unresolved
    );
    ensure!(
        program.modules.keys().cloned().collect::<BTreeSet<_>>() == *modules
            && program.animations.keys().cloned().collect::<BTreeSet<_>>() == *animations,
        "managed program catalog differs"
    );
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private original assets: managed immutable-program reconstruction"]
    fn relinks_exact_original_program() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| -> Result<()> {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../game-data/cabinet-extracted");
                let index = Index::scan(&root)?;
                let session =
                    Session::load(&index, "UserData/Houses/House07.iff", Options::default())?;
                let saved = session.save_managed()?;
                let content = saved.managed.unwrap();
                let mounted = content.owned.mount(&index)?;
                let program = link(
                    &mounted,
                    content.catalog,
                    &saved.modules,
                    &saved.animations,
                    saved.headline_catalog,
                    &saved.normalized_object_definitions,
                )?;
                fn difference(
                    a: &serde_json::Value,
                    b: &serde_json::Value,
                    path: &str,
                ) -> Option<String> {
                    if a == b {
                        return None;
                    }
                    match (a, b) {
                        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                            for k in a.keys().chain(b.keys()) {
                                if let Some(d) = difference(
                                    a.get(k).unwrap_or(&serde_json::Value::Null),
                                    b.get(k).unwrap_or(&serde_json::Value::Null),
                                    &format!("{path}/{k}"),
                                ) {
                                    return Some(d);
                                }
                            }
                        }
                        (serde_json::Value::Array(a), serde_json::Value::Array(b))
                            if a.len() == b.len() =>
                        {
                            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                                if let Some(d) = difference(a, b, &format!("{path}/{i}")) {
                                    return Some(d);
                                }
                            }
                        }
                        _ => {}
                    }
                    Some(format!("{path}: original={a}, relinked={b}"))
                }
                if let Some(d) = difference(
                    &serde_json::to_value(&session.program)?,
                    &serde_json::to_value(&program)?,
                    "program",
                ) {
                    anyhow::bail!("{d}");
                }
                ensure!(
                    session.program.fingerprint() == program.fingerprint(),
                    "program differs"
                );
                Ok(())
            })?
            .join()
            .expect("managed relink panicked")
    }
}
