//! Unpublished composition of explicitly supplied live Simulation/Session state.
//!
//! The caller first builds its VM (for example with retained_completed), then
//! supplies the complete Simulation snapshot around that VM. This seam does
//! not reconstruct those owners from an Incoming/starting save or substitute a
//! default Character, clock, routing driver, lifecycle, View or Options.
//! Structural composition is not authenticated native-baseline admission.
use super::{Options, Session, View};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use sims_content::{Index, character_loading::live, owned_resources::Neighborhood, saves::House};
use sims_sim::Snapshot;
use sims_vm::Program;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Rebind provider identities from canonical, version-checked source owners.
/// No installed mutable file is read to satisfy a missing embedded resource.
pub(super) fn bind_aliases(
    mut index: Index,
    aliases: Option<&BTreeMap<String, String>>,
) -> Result<Index> {
    let Some(aliases) = aliases else {
        return Ok(index);
    };
    ensure!(
        aliases.len() <= 4096,
        "completed provider alias extent exceeds bound"
    );
    index
        .resources
        .retain(|r| !r.locator.starts_with("native-provider:"));
    for (name, target) in aliases {
        let suffix = name
            .strip_prefix("native-provider:")
            .context("unsupported completed provider alias")?;
        ensure!(
            suffix.len() == 8
                && suffix.bytes().all(|b| b.is_ascii_hexdigit())
                && !target.starts_with("native-provider:"),
            "invalid/recursive completed provider alias"
        );
        let mut resource = index.resources[index.find(target)?].clone();
        ensure!(
            resource.locator.eq_ignore_ascii_case(target),
            "completed provider target must be a full canonical locator"
        );
        ensure!(
            !mutable_locator(target) || resource.owned.is_some(),
            "completed alias mutable target is not owned"
        );
        resource.locator = name.clone();
        // A binding is addressable by its exact provider identity. It is not
        // a second catalog file: basename/extension enumeration must continue
        // to see only the canonical resource, or lazy name lookups become
        // ambiguous and original resource precedence is changed.
        resource.name = name.clone();
        index.resources.push(resource);
    }
    Ok(index)
}

/// Every owner is explicit. No Deserialize implementation silently inherits
/// historical serde defaults from the existing saved formats. A native input
/// projector must independently establish every supplied owner/known absence.
pub struct Input {
    pub immutable: Arc<Index>,
    /// Current owned bytes, potentially different from House.sources' historical
    /// starting-input receipts. Only actually supplied/used files are required.
    pub mutable: Neighborhood,
    pub house: House,
    pub program: Program,
    pub simulation: Snapshot,
    pub assets: live::Snapshot,
    pub view: View,
    pub options: Options,
}

pub(super) fn mutable_locator(locator: &str) -> bool {
    let Some((directory, _)) = locator.split_once('/') else {
        return false;
    };
    directory
        .to_ascii_lowercase()
        .strip_prefix("userdata")
        .is_some_and(|tail| tail.bytes().all(|b| b.is_ascii_digit()))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Do not permit a fallback into another starting-save namespace. This tests
/// supplied Index metadata; it never opens one of those disk paths.
pub(super) fn immutable_index(index: &Index) -> Result<()> {
    ensure!(
        !index.resources.iter().any(|r| mutable_locator(&r.locator)),
        "completed immutable index contains a mutable starting-save namespace"
    );
    Ok(())
}

fn owned_resource(index: &Index, source: &str) -> Result<String> {
    let r = &index.resources[index.find(source)?];
    let bytes = r
        .owned
        .as_ref()
        .with_context(|| format!("completed mutable resource has no owned bytes: {source}"))?;
    ensure!(
        bytes.len() as u64 == r.size && r.size == r.unpacked_size,
        "completed owned resource extent differs: {source}"
    );
    Ok(digest(bytes))
}

pub(super) fn resources(
    index: &Index,
    source: &str,
    neighborhood: &str,
    program: &Program,
) -> Result<()> {
    // Current owned bytes are not required to equal historical House.sources
    // hashes. Parsed metadata, live World and the current resource namespace
    // have separate owners; no one is rebuilt from the others here.
    ensure!(
        mutable_locator(source) && mutable_locator(neighborhood),
        "completed House mutable source identity is unqualified"
    );
    owned_resource(index, source)?;
    owned_resource(index, neighborhood)?;
    for (name, module) in &program.modules {
        ensure!(
            !module.content_hash.is_empty(),
            "completed linked module source hash is unknown: {name}"
        );
        let current = if mutable_locator(name) {
            owned_resource(index, name)?
        } else {
            let r = &index.resources[index.find(name)?];
            digest(&r.read()?)
        };
        if mutable_locator(name) {
            ensure!(
                current == module.content_hash,
                "completed resident mutable module/current resource version mapping unsupported: {name}"
            );
        } else {
            ensure!(
                current == module.content_hash,
                "completed immutable linked module source hash differs: {name}"
            );
        }
    }
    Ok(())
}

fn simulation(program: &Program, snapshot: Snapshot) -> Result<sims_sim::Simulation> {
    ensure!(
        snapshot.state.native_scheduler.is_some() && snapshot.state.fixture_driver.is_none(),
        "completed Session needs the supplied original registry driver"
    );
    snapshot.restore(program).map_err(anyhow::Error::msg)
}

fn house_geometry(width: usize, height: usize, floors: usize) -> Result<()> {
    ensure!(
        width == height && (3..=64).contains(&width) && floors == 2,
        "completed House metadata geometry unavailable"
    );
    Ok(())
}

impl Session {
    /// Compose an independently owned unpublished Session and reuse all current
    /// validators. No Reader/load/bootstrap/enable_*, callbacks, RNG draws or
    /// animation-table repair executes. Host restoration links explicitly
    /// supplied resources; it is not evidence of native source/provider identity.
    pub fn from_completed_owned(input: Input) -> Result<Self> {
        // Same parsed-metadata guard as managed restore. Do not compare this
        // historic metadata to, or rebuild, the evolving live lot architecture.
        house_geometry(
            input.house.width,
            input.house.height,
            input.house.floors.len(),
        )?;
        immutable_index(&input.immutable)?;
        input.mutable.validate()?;
        ensure!(
            input.house.neighborhood_source
                == format!("{}/Neighborhood.iff", input.mutable.directory),
            "completed mutable namespace differs from supplied House"
        );
        ensure!(
            input.assets.neighborhood_source == input.house.neighborhood_source,
            "completed asset neighborhood owner differs"
        );
        let index = Arc::new(bind_aliases(
            input.mutable.mount(&input.immutable)?,
            input.view.resource_aliases.as_ref(),
        )?);
        resources(
            &index,
            &input.house.source,
            &input.house.neighborhood_source,
            &input.program,
        )?;
        let before_simulation = postcard::to_allocvec(&input.simulation.state)?;
        let before_assets = postcard::to_allocvec(&input.assets)?;
        let before_view = postcard::to_allocvec(&input.view)?;
        let before_options = postcard::to_allocvec(&input.options)?;
        let before_house = postcard::to_allocvec(&input.house)?;
        let before_program = input.program.fingerprint();
        let simulation = simulation(&input.program, input.simulation)?;
        let host = live::Host::restore(&index, input.view, input.assets)?;
        let mut session = Self {
            loaded_input_origin: None,
            index,
            managed_policy: super::managed::Policy::CompletedOwnedSchema2,
            house: input.house,
            program: input.program,
            simulation,
            host,
            options: input.options,
        };
        // Existing validate_restored compares update_contexts' candidate
        // Simulation before accepting it. A failure drops this unpublished owner.
        session.validate_restored()?;
        ensure!(
            postcard::to_allocvec(&session.simulation)? == before_simulation,
            "completed composition changed supplied Simulation state"
        );
        ensure!(
            postcard::to_allocvec(&session.host.snapshot())? == before_assets,
            "completed composition changed supplied asset owners"
        );
        ensure!(
            postcard::to_allocvec(&session.host.presentation)? == before_view,
            "completed composition changed supplied View owners"
        );
        ensure!(
            postcard::to_allocvec(&session.options)? == before_options,
            "completed composition changed supplied Options"
        );
        ensure!(
            postcard::to_allocvec(&session.house)? == before_house,
            "completed composition changed supplied House metadata"
        );
        ensure!(
            session.program.fingerprint() == before_program,
            "completed composition changed supplied Program"
        );
        Ok(session)
    }
}

#[cfg(test)]
mod tests;
