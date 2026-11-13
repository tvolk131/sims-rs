//! Versioned, simulation-agnostic fixture descriptions. Integrity is not parity.
use crate::BlobRef;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const CASE_SCHEMA: u32 = 1;
const MAX_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormat {
    Raw,
    Json,
    MessagePack,
    /// Recognized typed MessagePack trace index; dependency closure is verified.
    TraceIndex,
    /// Recognized byte-exact selected-file inventory with a declared blob closure.
    ArchiveInventory,
    Png,
    Jpeg,
    Other(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyMode {
    SelfContained,
    Declared,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub blob: BlobRef,
    pub format: ArtifactFormat,
    /// Version of the declared artifact payload, independent of case/storage versions.
    pub schema: u32,
    /// Complete flattened dependency closure; storage/GC never guesses inside payloads.
    pub dependencies: Vec<BlobRef>,
    pub dependency_mode: DependencyMode,
}
impl Artifact {
    pub fn self_contained(blob: BlobRef, format: ArtifactFormat, schema: u32) -> Self {
        Self {
            blob,
            format,
            schema,
            dependencies: vec![],
            dependency_mode: DependencyMode::SelfContained,
        }
    }
    pub fn declared(
        blob: BlobRef,
        format: ArtifactFormat,
        schema: u32,
        dependencies: Vec<BlobRef>,
    ) -> Self {
        Self {
            blob,
            format,
            schema,
            dependencies,
            dependency_mode: DependencyMode::Declared,
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.blob.validate()?;
        ensure!(
            self.dependency_mode != DependencyMode::SelfContained || self.dependencies.is_empty(),
            "self-contained artifact declares dependencies"
        );
        ensure!(
            !matches!(
                self.format,
                ArtifactFormat::TraceIndex | ArtifactFormat::ArchiveInventory
            ) || self.dependency_mode == DependencyMode::Declared,
            "trace/archive index requires declared dependencies"
        );
        ensure!(
            self.schema > 0,
            "artifact payload schema must be explicit/nonzero"
        );
        ensure!(
            self.dependencies.len() <= MAX_ENTRIES,
            "too many artifact dependencies"
        );
        if let ArtifactFormat::Other(name) = &self.format {
            text(name, 128, "artifact format")?;
        }
        validate_refs(std::iter::once(&self.blob).chain(&self.dependencies))
    }
    pub fn blob_refs(&self) -> Vec<BlobRef> {
        unique_refs(std::iter::once(&self.blob).chain(&self.dependencies))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Authored,
    Imported,
    Observed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HashVerification {
    Provided,
    Verified { method: String, report: Artifact },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceSource {
    pub label: String,
    pub sha256: String,
    pub verification: HashVerification,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub origin: Origin,
    /// An inert locator, never automatically opened or treated as an authority.
    pub source_locator: Option<String>,
    pub sources: Vec<ProvenanceSource>,
    /// Exact argument vectors, inert evidence; this library never executes them.
    pub commands: Vec<Vec<String>>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Coverage {
    Unqualified {
        reason: String,
    },
    Component {
        scope: String,
        limitations: Vec<String>,
    },
    /// An explicit human/tool claim with its report, not certification by storage.
    ClaimedParity {
        contract: String,
        report: Artifact,
        limitations: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Boundary {
    Unspecified,
    CompletedTick { tick: u64 },
    Phase { name: String, tick: Option<u64> },
    Event { name: String, tick: Option<u64> },
}
impl Boundary {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Phase { name, .. } | Self::Event { name, .. } => {
                text(name, 1024, "boundary name")
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InputEvent {
    pub name: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScheduleStep {
    Input {
        input: InputEvent,
    },
    /// Counts completed simulation updates, not callbacks, slices or wall time.
    WaitCompletedTicks {
        count: u64,
    },
    /// This boundary must be reached explicitly; it implies no completed tick.
    WaitBoundary {
        boundary: Boundary,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Observed,
    Resumable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub label: String,
    pub kind: ReferenceKind,
    pub artifact: Artifact,
    /// Required only for a declared resumable reference; never inferred from format.
    pub resume_contract: Option<String>,
    pub boundary: Boundary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Screenshot {
    pub label: String,
    pub artifact: Artifact,
    pub boundary: Boundary,
    pub logical_width: u32,
    pub logical_height: u32,
    pub scale_factor: f64,
    pub physical_width: u32,
    pub physical_height: u32,
    pub camera: Option<Artifact>,
    pub ui_configuration: Option<Artifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CaseManifest {
    pub schema: u32,
    pub id: String,
    pub title: String,
    pub source: Artifact,
    pub artifacts: Vec<Artifact>,
    pub provenance: Provenance,
    pub coverage: Coverage,
    pub schedule: Vec<ScheduleStep>,
    pub references: Vec<Reference>,
    pub screenshots: Vec<Screenshot>,
    pub notes: Vec<String>,
}
impl CaseManifest {
    /// Imports are observations until an explicit separate contract/report says more.
    pub fn minimal_import(
        id: impl Into<String>,
        title: impl Into<String>,
        source: Artifact,
        notes: Vec<String>,
    ) -> Self {
        Self {
            schema: CASE_SCHEMA,
            id: id.into(),
            title: title.into(),
            source,
            artifacts: vec![],
            provenance: Provenance {
                origin: Origin::Imported,
                source_locator: None,
                sources: vec![],
                commands: vec![],
                notes: vec![],
            },
            coverage: Coverage::Unqualified {
                reason: "Imported evidence; no semantic or resume qualification performed".into(),
            },
            schedule: vec![],
            references: vec![],
            screenshots: vec![],
            notes,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == CASE_SCHEMA,
            "unsupported case schema {}",
            self.schema
        );
        validate_id(&self.id)?;
        text(&self.title, 1024, "case title")?;
        self.source.validate()?;
        for length in [
            self.artifacts.len(),
            self.schedule.len(),
            self.references.len(),
            self.screenshots.len(),
        ] {
            ensure!(length <= MAX_ENTRIES, "case collection too large");
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        if let Some(locator) = &self.provenance.source_locator {
            text(locator, 65_536, "source locator")?;
        }
        ensure!(
            self.provenance.sources.len() <= MAX_ENTRIES
                && self.provenance.commands.len() <= MAX_ENTRIES,
            "provenance collection too large"
        );
        let mut source_labels = std::collections::BTreeSet::new();
        for source in &self.provenance.sources {
            text(&source.label, 1024, "source label")?;
            ensure!(
                source_labels.insert(&source.label),
                "duplicate provenance source label"
            );
            ensure!(
                source.sha256.len() == 64
                    && source
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid source SHA-256"
            );
            if let HashVerification::Verified { method, report } = &source.verification {
                text(method, 65_536, "source verification method")?;
                report.validate()?;
            }
        }
        for command in &self.provenance.commands {
            ensure!(
                !command.is_empty() && command.len() <= 65_536,
                "invalid provenance argv"
            );
            text(&command[0], 65_536, "command executable")?;
            ensure!(
                command
                    .iter()
                    .all(|arg| arg.len() <= 65_536 && !arg.contains('\0')),
                "invalid command argument"
            );
        }
        notes(&self.provenance.notes)?;
        notes(&self.notes)?;
        match &self.coverage {
            Coverage::Unqualified { reason } => text(reason, 65_536, "unqualified reason")?,
            Coverage::Component { scope, limitations } => {
                text(scope, 65_536, "component scope")?;
                notes(limitations)?;
            }
            Coverage::ClaimedParity {
                contract,
                report,
                limitations,
            } => {
                text(contract, 65_536, "claimed parity contract")?;
                report.validate()?;
                notes(limitations)?;
            }
        }
        for step in &self.schedule {
            match step {
                ScheduleStep::Input { input } => text(&input.name, 1024, "input name")?,
                ScheduleStep::WaitCompletedTicks { count } => {
                    ensure!(*count > 0, "completed-tick wait must be positive")
                }
                ScheduleStep::WaitBoundary { boundary } => {
                    boundary.validate()?;
                    ensure!(
                        !matches!(boundary, Boundary::Unspecified),
                        "boundary wait cannot be unspecified"
                    );
                }
            }
        }
        for reference in &self.references {
            text(&reference.label, 1024, "reference label")?;
            reference.artifact.validate()?;
            reference.boundary.validate()?;
            match (&reference.kind, &reference.resume_contract) {
                (ReferenceKind::Observed, None) => (),
                (ReferenceKind::Resumable, Some(contract)) => {
                    text(contract, 65_536, "resume contract")?
                }
                _ => bail!("reference resume contract contradicts observed/resumable kind"),
            }
        }
        for screenshot in &self.screenshots {
            text(&screenshot.label, 1024, "screenshot label")?;
            screenshot.artifact.validate()?;
            screenshot.boundary.validate()?;
            for configuration in [&screenshot.camera, &screenshot.ui_configuration]
                .into_iter()
                .flatten()
            {
                configuration.validate()?;
            }
            ensure!(
                screenshot.logical_width > 0
                    && screenshot.logical_height > 0
                    && screenshot.physical_width > 0
                    && screenshot.physical_height > 0
                    && screenshot.scale_factor.is_finite()
                    && screenshot.scale_factor > 0.0,
                "invalid screenshot dimensions or scale"
            );
        }
        validate_refs(self.blob_refs().iter())
    }
    /// Every structured Artifact role, including configuration/verification reports.
    pub fn all_artifacts(&self) -> Vec<&Artifact> {
        let mut artifacts = vec![&self.source];
        artifacts.extend(&self.artifacts);
        artifacts.extend(self.references.iter().map(|r| &r.artifact));
        for screenshot in &self.screenshots {
            artifacts.push(&screenshot.artifact);
            artifacts.extend(screenshot.camera.iter());
            artifacts.extend(screenshot.ui_configuration.iter());
        }
        if let Coverage::ClaimedParity { report, .. } = &self.coverage {
            artifacts.push(report);
        }
        for source in &self.provenance.sources {
            if let HashVerification::Verified { report, .. } = &source.verification {
                artifacts.push(report);
            }
        }
        artifacts
    }
    /// Complete declared closure, deduplicated deterministically by content identity.
    pub fn blob_refs(&self) -> Vec<BlobRef> {
        let mut refs: Vec<_> = self
            .all_artifacts()
            .into_iter()
            .flat_map(Artifact::blob_refs)
            .collect();
        // Keep contradictory lengths visible to validate rather than hiding them.
        refs.sort_by(|a, b| (&a.sha256, a.bytes).cmp(&(&b.sha256, b.bytes)));
        refs.dedup_by(|a, b| a.sha256 == b.sha256 && a.bytes == b.bytes);
        refs
    }
}

pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id != "."
            && id != ".."
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "invalid case id"
    );
    Ok(())
}
fn text(value: &str, maximum: usize, name: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= maximum && !value.contains('\0'),
        "invalid {name}"
    );
    Ok(())
}
fn notes(values: &[String]) -> Result<()> {
    ensure!(values.len() <= MAX_ENTRIES, "too many notes");
    for note in values {
        text(note, 65_536, "note")?;
    }
    Ok(())
}
fn validate_refs<'a>(refs: impl Iterator<Item = &'a BlobRef>) -> Result<()> {
    let mut lengths = BTreeMap::new();
    for reference in refs {
        reference.validate()?;
        if let Some(old) = lengths.insert(&reference.sha256, reference.bytes) {
            ensure!(
                old == reference.bytes,
                "one blob digest has conflicting lengths"
            );
        }
    }
    Ok(())
}
fn unique_refs<'a>(refs: impl Iterator<Item = &'a BlobRef>) -> Vec<BlobRef> {
    let mut refs: Vec<_> = refs.cloned().collect();
    refs.sort_by(|a, b| (&a.sha256, a.bytes).cmp(&(&b.sha256, b.bytes)));
    refs.dedup_by(|a, b| a.sha256 == b.sha256 && a.bytes == b.bytes);
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    fn blob(n: char) -> BlobRef {
        BlobRef {
            sha256: n.to_string().repeat(64),
            bytes: 12,
        }
    }
    fn artifact(n: char) -> Artifact {
        Artifact::self_contained(blob(n), ArtifactFormat::Raw, 1)
    }
    fn case() -> CaseManifest {
        CaseManifest::minimal_import("import-1", "Observed input", artifact('a'), vec![])
    }
    #[test]
    fn imported_storage_never_qualifies_or_claims_resume() {
        let c = case();
        c.validate().unwrap();
        assert!(matches!(c.coverage, Coverage::Unqualified { .. }));
        assert_eq!(c.provenance.origin, Origin::Imported);
        assert!(c.references.is_empty());
    }
    #[test]
    fn unknown_version_fields_and_path_ids_fail() {
        let mut c = case();
        c.schema = 2;
        assert!(c.validate().is_err());
        c = case();
        c.id = "../outside".into();
        assert!(c.validate().is_err());
        let mut value = serde_json::to_value(case()).unwrap();
        value["future"] = Value::Bool(true);
        assert!(serde_json::from_value::<CaseManifest>(value).is_err());
    }
    #[test]
    fn closure_includes_every_artifact_role_and_conflicts_reject() {
        let mut c = case();
        c.source.dependency_mode = DependencyMode::Declared;
        c.source.dependencies.push(blob('b'));
        c.artifacts.push(artifact('c'));
        c.references.push(Reference {
            label: "observation".into(),
            kind: ReferenceKind::Observed,
            artifact: artifact('d'),
            resume_contract: None,
            boundary: Boundary::Unspecified,
        });
        c.coverage = Coverage::ClaimedParity {
            contract: "explicit external claim only".into(),
            report: artifact('e'),
            limitations: vec![],
        };
        c.screenshots.push(Screenshot {
            label: "frame".into(),
            artifact: artifact('f'),
            boundary: Boundary::Unspecified,
            logical_width: 100,
            logical_height: 50,
            scale_factor: 1.5,
            physical_width: 150,
            physical_height: 75,
            camera: None,
            ui_configuration: None,
        });
        c.validate().unwrap();
        assert_eq!(c.blob_refs().len(), 6);
        let mut conflict = blob('b');
        conflict.bytes = 13;
        c.artifacts[0].dependency_mode = DependencyMode::Declared;
        c.artifacts[0].dependencies.push(conflict);
        assert!(c.validate().is_err());
    }
    #[test]
    fn structured_provenance_and_configuration_closure_are_explicit() {
        let mut c = case();
        c.provenance.sources.push(ProvenanceSource {
            label: "provided original image".into(),
            sha256: "a".repeat(64),
            verification: HashVerification::Provided,
        });
        c.provenance.sources.push(ProvenanceSource {
            label: "checked source".into(),
            sha256: "b".repeat(64),
            verification: HashVerification::Verified {
                method: "explicit checksum report".into(),
                report: artifact('c'),
            },
        });
        c.provenance
            .commands
            .push(vec!["collector".into(), "--optional".into(), String::new()]);
        c.screenshots.push(Screenshot {
            label: "frame".into(),
            artifact: artifact('d'),
            boundary: Boundary::Unspecified,
            logical_width: 10,
            logical_height: 10,
            scale_factor: 2.0,
            physical_width: 20,
            physical_height: 20,
            camera: Some(artifact('e')),
            ui_configuration: Some(artifact('f')),
        });
        c.validate().unwrap();
        assert_eq!(c.blob_refs().len(), 5);
        assert_eq!(c.all_artifacts().len(), 5);
        let encoded = serde_json::to_vec(&c).unwrap();
        let back: CaseManifest = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(back, c);
        c.provenance.sources[1].sha256 = "future nonhash".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn wait_boundaries_and_resume_contracts_are_explicit() {
        let mut c = case();
        c.schedule = vec![
            ScheduleStep::WaitCompletedTicks { count: 2 },
            ScheduleStep::WaitBoundary {
                boundary: Boundary::Phase {
                    name: "menu_ready".into(),
                    tick: Some(9),
                },
            },
        ];
        c.references.push(Reference {
            label: "checkpoint".into(),
            kind: ReferenceKind::Resumable,
            artifact: artifact('b'),
            resume_contract: None,
            boundary: Boundary::Unspecified,
        });
        assert!(c.validate().is_err());
        c.references[0].resume_contract = Some("engine-v1 explicit known owners".into());
        c.validate().unwrap();
        c.schedule.push(ScheduleStep::WaitBoundary {
            boundary: Boundary::Unspecified,
        });
        assert!(c.validate().is_err());
    }
}
