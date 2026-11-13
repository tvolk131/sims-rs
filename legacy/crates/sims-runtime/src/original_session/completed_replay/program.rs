//! Independently link the exact captured resident Program from explicit data.
use crate::{
    conformance,
    original_session::{self, completed_world},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_content::Index;
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramInputs {
    pub reentry: Value,
    pub systems: Value,
    pub appearances: Value,
    pub bindings: Value,
    pub qualification: Value,
    pub catalog_bindings: Value,
    pub corrected_animation_input: Option<Value>,
    pub sample_buffer_proof: Option<Value>,
    pub resident_global_capture: Option<Value>,
}
impl ProgramInputs {
    pub fn read(config: &Value, mut read: impl FnMut(&str) -> Result<Value>) -> Result<Self> {
        let mut required = |k: &str| {
            read(
                config[k]
                    .as_str()
                    .context("completed Program source absent")?,
            )
        };
        let reentry = required("reentry")?;
        let systems = required("systems")?;
        let appearances = required("appearances")?;
        let bindings = required("bindings")?;
        let qualification = required("qualification")?;
        let catalog_bindings = required("catalog_bindings")?;
        let mut optional = |k: &str| config[k].as_str().map(&mut read).transpose();
        Ok(Self {
            reentry,
            systems,
            appearances,
            bindings,
            qualification,
            catalog_bindings,
            corrected_animation_input: optional("corrected_animation_input")?,
            sample_buffer_proof: optional("sample_buffer_proof")?,
            resident_global_capture: optional("resident_global_capture")?,
        })
    }
    pub fn link(
        &self,
        owned: &sims_content::owned_resources::Neighborhood,
        immutable: &Index,
    ) -> Result<(sims_vm::Program, BTreeMap<String, String>, String, Value)> {
        let reentry = self.reentry.clone();
        let systems = self.systems.clone();
        let appearances = self.appearances.clone();
        let parts = conformance::completed_reentry::import_parts(reentry.clone())?;
        let (mut program, _) =
            completed_world::program::join(parts.program, &systems["body_profiles"])?;
        program.animations =
            conformance::completed_animations::import_parts(
                &appearances["prototypes_input"]["resident_input"]["skin_input"]["skeleton_input"]
                    ["animation_input"],
            )?
            .assets;
        let unversioned_program = program.fingerprint();
        if let Some(corrected) = &self.corrected_animation_input {
            let proof = self
                .sample_buffer_proof
                .as_ref()
                .context("corrected sample proof absent")?;
            original_session::completed_animations::validate_sample_replacement(
                &appearances["prototypes_input"]["resident_input"]["skin_input"]["skeleton_input"]
                    ["animation_input"],
                corrected,
                proof,
            )?;
            program.animations = conformance::completed_animations::import_parts(corrected)?.assets;
        }
        let index = owned.mount(immutable)?;
        let bindings = self.bindings.clone();
        let qualification = self.qualification.clone();
        let mut aliases = BTreeMap::new();
        let catalog = self.catalog_bindings.clone();
        ensure!(
            catalog["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
                && catalog["provenance"]["observation_sha256"]
                    == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
            "retained catalog source epoch differs"
        );
        for b in bindings["candidate_module_bindings"]
            .as_array()
            .context("provider bindings absent")?
            .iter()
            .chain(
                catalog["candidate_module_bindings"]
                    .as_array()
                    .context("retained catalog bindings absent")?,
            )
        {
            let source = b["native_source"]
                .as_str()
                .context("native source absent")?;
            let locator = b["ordered_component_locators"][0]
                .as_str()
                .context("primary source absent")?;
            if locator.to_ascii_lowercase().starts_with("userdata/") {
                let q: Vec<_> = qualification["qualified_private_module_bindings"]
                    .as_array()
                    .context("private qualification absent")?
                    .iter()
                    .filter(|q| q["native_source"] == source)
                    .collect();
                ensure!(
                    q.len() == 1
                        && q[0]["complete_native_source_version_qualified"] == true
                        && q[0]["primary_content_sha256"] == b["primary_content_sha256"],
                    "private source epoch unqualified"
                );
            }
            let resource = &index.resources[index.find(locator)?];
            let hash = sims_fixtures::digest(&resource.read()?);
            ensure!(
                b["primary_content_sha256"] == hash,
                "whole source version differs"
            );
            if let Some(module) = program.modules.get_mut(source) {
                ensure!(
                    module.content_hash.is_empty() || module.content_hash == hash,
                    "resident module already records another version"
                );
                module.content_hash = hash.clone();
            } else {
                ensure!(
                    program
                        .resource_storage
                        .as_ref()
                        .is_some_and(|defs| defs.iter().any(|d| d.source == source
                            && b["guid"] == d.guid
                            && b["object"] == d.object
                            && b["cached_definition_bytes"] == 216)),
                    "bound provider absent from retained Program/catalog"
                );
            }
            if let Some(defs) = &mut program.resource_storage {
                for d in defs.iter_mut().filter(|d| d.source == source) {
                    ensure!(
                        d.content_hash.is_empty() || d.content_hash == hash,
                        "resource storage source version differs"
                    );
                    d.content_hash = hash.clone();
                }
            }
            ensure!(
                aliases
                    .insert(source.into(), resource.locator.clone())
                    .is_none(),
                "duplicate provider binding"
            );
        }
        ensure!(
            aliases.len()
                == program.modules.len()
                    + catalog["candidate_module_bindings"]
                        .as_array()
                        .unwrap()
                        .len(),
            "Program provider closure incomplete"
        );
        if let Some(global) = &self.resident_global_capture {
            original_session::completed_assets::import_resident_global_routines(
                global,
                &index,
                &aliases[&program.globals],
                &mut program,
            )?;
        }
        Ok((
            program,
            aliases,
            unversioned_program,
            reentry["source_closure"].clone(),
        ))
    }
}
