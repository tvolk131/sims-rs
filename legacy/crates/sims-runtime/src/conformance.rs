//! Explicit, headless fixture suites. Independent cases may run in separate
//! workers; their simulation order and RNG ownership are never parallelized.
use crate::fixture_replay::Difference;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod component;
pub mod completed_stacks;
pub mod completed_animations;
pub mod completed_groups;
pub mod completed_person_world;
pub mod completed_reentry;
pub mod completed_skeletons;
pub mod completed_skin;
pub mod completed_residents;
pub mod completed_prototypes;
pub mod completed_appearances;
pub mod completed_ordinary;
pub mod runner;
pub mod native_household;
pub mod session;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub schema: u32,
    pub id: String,
    pub cases: Vec<CaseSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub store: PathBuf,
    pub case_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Adapter {
    SavedRng,
    CompletedStacks,
    CompletedAnimations,
    CompletedGroups,
    CompletedPersonWorld,
    CompletedReentry,
    CompletedSkeletons,
    CompletedSkin,
    CompletedResidents,
    CompletedPrototypes,
    CompletedAppearances { immutable_content:PathBuf },
    CompletedOrdinary { immutable_content:PathBuf },
    SchedulerClock,
    ManagedSession { immutable_content: PathBuf },
    NativeHousehold {
        house: String,
        #[serde(default)]
        immutable_content: Option<PathBuf>,
        #[serde(default)]
        checks: native_household::Checks,
    },
    EvidenceOnly { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseSpec {
    pub id: String,
    pub adapter: Adapter,
    pub fixture: Option<Fixture>,
    pub required: bool,
    pub timeout_seconds: u64,
    /// Suite acceptance requirement; component cases use zero, not tick credit.
    #[serde(default)]
    pub minimum_matched_ticks: u64,
    /// Admission weight, not a promise of an OS-enforced memory ceiling.
    pub estimated_memory_mib: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Diverged,
    Incomplete,
    Unsupported,
    EvidenceOnly,
    Error,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    OriginalComponent,
    RustContinuation,
    OriginalHousehold,
    ObservationOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterResult {
    pub status: Status,
    pub scope: Scope,
    pub contract: String,
    pub observations: u64,
    pub completed_ticks: u64,
    pub matched_completed_ticks: u64,
    pub difference: Option<Difference>,
    pub detail: Option<String>,
    /// Paths relative to this worker's new output directory.
    pub artifacts: Vec<PathBuf>,
}

impl AdapterResult {
    pub fn stopped(status: Status, scope: Scope, detail: impl Into<String>) -> Self {
        Self {
            status,
            scope,
            contract: "unavailable".into(),
            observations: 0,
            completed_ticks: 0,
            matched_completed_ticks: 0,
            difference: None,
            detail: Some(detail.into()),
            artifacts: vec![],
        }
    }
}
