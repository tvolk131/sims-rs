use super::super::super::service_capture::{Captured, Opening, OpeningCut, Request, Selection};
use super::*;
use sims_vm::Location;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cut {
    Animation,
    Creation,
    Placement,
}

impl Cut {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "animation" => Ok(Self::Animation),
            "creation" => Ok(Self::Creation),
            "placement" => Ok(Self::Placement),
            _ => anyhow::bail!("unsupported package-opening restore cut: {value}"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Animation => "animation",
            Self::Creation => "creation",
            Self::Placement => "placement",
        }
    }
}

fn package_request(origin: &Location, caller: i16, actor: i16) -> bool {
    caller == actor && origin.source.ends_with("::MagicStarterKit.iff")
}

fn exact_restore_proof(evidence: &Value) -> bool {
    evidence["exact_simulation_equal"] == true && evidence["exact_checkpoint_equal"] == true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Continuation {
    pub serial: u32,
    pub before_contents: Vec<Vec<i16>>,
    pub package_dialog_seen: bool,
}

impl Continuation {
    pub(super) fn from_receipt(prior: &Value) -> Result<Self> {
        let evidence = &prior["opening_restore"]["evidence"];
        Cut::parse(evidence["cut"].as_str().context("opening cut absent")?)?;
        ensure!(
            exact_restore_proof(evidence),
            "opening cut lacks its exact serialized-restore proof"
        );
        ensure!(
            evidence["process_id"]
                .as_u64()
                .is_some_and(|process| process > 0 && process != u64::from(std::process::id())),
            "opening cut was not recorded by a distinct process"
        );
        let context: Self = serde_json::from_value(evidence["continuation"].clone())?;
        ensure!(
            context.before_contents.len() == CONTENTS.len() && evidence["serial"] == context.serial,
            "opening continuation metadata differs"
        );
        Ok(context)
    }

    pub(super) fn validate_active(
        &self,
        app: &Application,
        actor: i16,
        package: &Identity,
    ) -> Result<()> {
        let active = app
            .session()?
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|actions| actions.actors.get(&actor))
            .and_then(|actions| actions.active.as_ref());
        ensure!(
            active.is_some_and(|action| {
                action.id == self.serial
                    && action.target == package.object
                    && action.routine == 4097
            }),
            "restored package Open action is not the recorded active submission"
        );
        Ok(())
    }
}

pub(super) struct Probe {
    cut: Option<Cut>,
    evidence: Option<Value>,
}

impl Probe {
    pub(super) fn pending(&self) -> bool {
        self.cut.is_some() && self.evidence.is_none()
    }

    pub(super) fn from_environment() -> Result<Self> {
        let cut = std::env::var("SIMS_MAGIC_PACKAGE_OPENING_RESTORE_CUT")
            .ok()
            .map(|value| Cut::parse(&value))
            .transpose()?;
        Ok(Self {
            cut,
            evidence: None,
        })
    }

    pub(super) fn request(
        &self,
        app: &Application,
        actor: &Identity,
        package: &Identity,
        continuation: &Continuation,
    ) -> Result<Option<Request>> {
        let Some(cut) = self.cut.filter(|_| self.evidence.is_none()) else {
            return Ok(None);
        };
        actor.check(app.session()?)?;
        package.check(app.session()?)?;
        let source = app
            .session()?
            .simulation
            .vm
            .world
            .objects
            .get(&package.object)
            .and_then(|object| object.definition.as_ref())
            .context("opening package definition absent")?
            .source
            .clone();
        ensure!(
            source.ends_with("::MagicStarterKit.iff"),
            "opening package source differs"
        );
        Ok(Some(Request::new(
            app.capture_frame(),
            Selection::Opening(Opening {
                cut: match cut {
                    Cut::Animation => OpeningCut::Animation,
                    Cut::Creation => OpeningCut::Creation,
                    Cut::Placement => OpeningCut::Placement,
                },
                actor: actor.clone(),
                package: package.clone(),
                serial: continuation.serial,
                source,
                contents: CONTENTS.to_vec(),
                continuation: serde_json::to_value(continuation)?,
            }),
        )))
    }

    pub(super) fn consume(
        &mut self,
        app: &mut Application,
        output: &Path,
        actor: &Identity,
        package: &Identity,
        captured: Captured,
    ) -> Result<()> {
        let cut = self.cut.context("opening capture was not requested")?;
        ensure!(
            self.evidence.is_none(),
            "opening restore proof already published"
        );
        let thread = match (cut, captured.cut) {
            (Cut::Animation, sims_sim::ServiceCut::AwaitingAnimation { thread, .. })
            | (Cut::Creation, sims_sim::ServiceCut::Creation { thread, .. })
            | (Cut::Placement, sims_sim::ServiceCut::FindLocation { thread, .. }) => thread,
            _ => anyhow::bail!("captured service cut differs from requested opening boundary"),
        };
        let continuation: Continuation = serde_json::from_value(captured.continuation.clone())?;
        let envelope: Value = serde_json::from_slice(&captured.checkpoint)?;
        let saved: Saved = serde_json::from_value(envelope["session"].clone())?;
        let bytes = captured.checkpoint;
        let path = output.join("during-opening.private.json");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?
            .write_all(&bytes)?;
        let persisted = std::fs::read(&path)?;
        ensure!(persisted == bytes, "persisted opening checkpoint differs");
        let restored = Application::restore_checkpoint(
            &app.viewer.args,
            app.session()?.index(),
            app.viewer.icons.as_ref().context("icons absent")?,
            app.epoch,
            &persisted,
        )?;
        let retained_routing =
            verify_retained_routing(restored.session()?, app.viewer.args.inferred_routing)?;
        actor.check(restored.session()?)?;
        package.check(restored.session()?)?;
        continuation.validate_active(&restored, actor.object, package)?;
        let equal = saved.simulation.state == restored.session()?.simulation;
        let restored_bytes = restored.checkpoint()?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join("during-opening-restored.private.json"))?
            .write_all(&restored_bytes)?;
        ensure!(equal, "opening restore changed exact simulation state");
        ensure!(
            persisted == restored_bytes,
            "opening restore changed the complete application checkpoint"
        );
        self.evidence = Some(json!({
            "cut":cut.name(),"actor":actor.object,"package":package,"serial":continuation.serial,
            "tick":saved.simulation.state.vm.tick,"thread":thread,"service_cut":captured.cut,"boundary":captured.boundary,
            "continuation":continuation,
            "retained_routing":retained_routing,
            "checkpoint_sha256":engine::digest(&bytes),
            "restored_checkpoint_sha256":engine::digest(&restored_bytes),
            "exact_simulation_equal":true,"exact_checkpoint_equal":true,
            "process_id":std::process::id(),
            "same_process_serialized_restore":true,"separate_process_resume":false,
            "native_qualification":false,
        }));
        *app = restored;
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<()> {
        ensure!(
            self.cut.is_none() || self.evidence.as_ref().is_some_and(exact_restore_proof),
            "requested opening restore boundary was not observed and exactly restored"
        );
        Ok(())
    }

    pub(super) fn report(&self) -> Value {
        json!({"requested":self.cut.map(Cut::name),"evidence":self.evidence})
    }
}

#[test]
fn opening_cut_names_are_explicit_and_unknown_cuts_fail() {
    for cut in [Cut::Animation, Cut::Creation, Cut::Placement] {
        assert_eq!(Cut::parse(cut.name()).unwrap(), cut);
    }
    for value in ["", "before-open", "Animation", "anything"] {
        assert!(Cut::parse(value).is_err());
    }
}

#[test]
fn opening_request_requires_the_package_source_and_selected_actor() {
    let mut origin = Location {
        source: "authored/archive::MagicStarterKit.iff".into(),
        bhav: 4100,
        instruction: 1,
    };
    assert!(package_request(&origin, 7, 7));
    assert!(!package_request(&origin, 8, 7));
    origin.source = "authored/archive::Other.iff".into();
    assert!(!package_request(&origin, 7, 7));
    origin.source = "authored/archive::MagicStarterKit.iff/Other.iff".into();
    assert!(!package_request(&origin, 7, 7));
}

#[test]
fn a_requested_cut_without_observed_boundary_cannot_pass() {
    assert!(
        Probe {
            cut: None,
            evidence: None
        }
        .finish()
        .is_ok()
    );
    assert!(
        Probe {
            cut: Some(Cut::Animation),
            evidence: None
        }
        .finish()
        .is_err()
    );
}

#[test]
fn simulation_only_restore_proof_cannot_finish_an_opening_cut() {
    for evidence in [
        json!({"exact_simulation_equal":true}),
        json!({"exact_simulation_equal":true,"exact_checkpoint_equal":false}),
        json!({"exact_simulation_equal":false,"exact_checkpoint_equal":true}),
        json!({"exact_simulation_equal":true,"exact_checkpoint_equal":"true"}),
    ] {
        assert!(
            Probe {
                cut: Some(Cut::Animation),
                evidence: Some(evidence),
            }
            .finish()
            .is_err()
        );
    }
    assert!(
        Probe {
            cut: Some(Cut::Animation),
            evidence: Some(json!({"exact_simulation_equal":true,"exact_checkpoint_equal":true})),
        }
        .finish()
        .is_ok()
    );
}

#[test]
fn opening_resume_requires_exact_proof_and_complete_consistent_context() {
    let mut prior = json!({"opening_restore":{"evidence":{
        "cut":"animation","serial":5,"exact_simulation_equal":true,"exact_checkpoint_equal":true,
        "continuation":{"serial":5,"before_contents":[[],[],[],[]],"package_dialog_seen":false}
    }}});
    prior["opening_restore"]["evidence"]["process_id"] = json!(u64::from(std::process::id()) + 1);
    let valid = prior.clone();
    assert!(Continuation::from_receipt(&prior).is_ok());
    prior["opening_restore"]["evidence"]["exact_simulation_equal"] = json!(false);
    assert!(Continuation::from_receipt(&prior).is_err());
    prior["opening_restore"]["evidence"]["exact_simulation_equal"] = json!(true);
    prior["opening_restore"]["evidence"]["exact_checkpoint_equal"] = json!(false);
    assert!(Continuation::from_receipt(&prior).is_err());
    prior["opening_restore"]["evidence"]
        .as_object_mut()
        .unwrap()
        .remove("exact_checkpoint_equal");
    assert!(Continuation::from_receipt(&prior).is_err());
    prior["opening_restore"]["evidence"]["exact_checkpoint_equal"] = json!(true);
    prior["opening_restore"]["evidence"]["serial"] = json!(6);
    assert!(Continuation::from_receipt(&prior).is_err());
    prior["opening_restore"]["evidence"]["serial"] = json!(5);
    prior["opening_restore"]["evidence"]["continuation"]["before_contents"] = json!([[]]);
    assert!(Continuation::from_receipt(&prior).is_err());
    prior["opening_restore"]["evidence"]["continuation"] = json!(null);
    assert!(Continuation::from_receipt(&prior).is_err());
    let mut same_process = valid;
    same_process["opening_restore"]["evidence"]["process_id"] = json!(std::process::id());
    assert!(Continuation::from_receipt(&same_process).is_err());
    same_process["opening_restore"]["evidence"]["process_id"] = json!(null);
    assert!(Continuation::from_receipt(&same_process).is_err());
}
