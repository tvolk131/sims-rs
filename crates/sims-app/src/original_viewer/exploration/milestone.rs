//! Read-only predicates. These never advance, repair, or reseed the game.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Milestone {
    CompletedTicks {
        count: u64,
    },
    Animation {
        actor: Identity,
        name: String,
    },
    /// A live, non-ended layer inside a half-open phase interval. Unlike
    /// Animation, a retained faded/ended clip cannot satisfy this checkpoint.
    AnimationPhase {
        actor: Identity,
        name: String,
        minimum: f32,
        maximum: f32,
    },
    Level {
        actor: Identity,
        value: i32,
    },
    ScriptFrame {
        actor: Identity,
        source: String,
        bhav: Option<u16>,
        present: bool,
    },
    PersonField {
        actor: Identity,
        field: i16,
        value: i16,
    },
    ActionActive {
        actor: Identity,
        target: i16,
    },
    Contained {
        actor: Identity,
        target: i16,
    },
    QueueEmpty {
        actor: Identity,
    },
    TargetAbsent {
        actor: Identity,
        target: i16,
    },
    /// Queue absence alone is insufficient: the actor must also release every
    /// live/pending action-use frame and physical containment of this target.
    InteractionReleased {
        actor: Identity,
        target: Identity,
    },
    ObjectUseCount {
        target: Identity,
        count: i16,
    },
    /// Shared furniture may still be used by another actor. Require its global
    /// count to equal all live and pending-unwind action-use records instead.
    ObjectUsesAccounted {
        target: Identity,
    },
    /// Both physical link directions agree and no other person claims this
    /// container. Useful for exclusive seats; not a universal multipart rule.
    SolePersonContained {
        actor: Identity,
        target: Identity,
    },
    /// The native previous-action result bit, not a generic claim of success.
    PreviousActionReturn {
        actor: Identity,
        target: Identity,
        returned: bool,
    },
    MenuReady {
        minimum_actions: usize,
    },
    NoMenu,
    Modal {
        source: String,
        title: String,
        body_sha256: String,
    },
    Selected {
        actor: Identity,
    },
    Mode {
        value: i32,
    },
    FurnitureEdit {
        expected: Box<buy_edit_cycle::Expected>,
    },
    All {
        conditions: Vec<Milestone>,
    },
    Any {
        conditions: Vec<Milestone>,
    },
}
impl Milestone {
    pub(super) fn reached(&self, app: &Application, start: u64) -> Result<bool> {
        let s = app.session()?;
        Ok(match self {
            Self::CompletedTicks { count } => s.simulation.vm.tick.saturating_sub(start) >= *count,
            Self::Animation { actor, name } => {
                actor.check(s)?;
                s.simulation.characters.get(&actor.object).is_some_and(|r| {
                    r.character
                        .skin
                        .animator
                        .layers()
                        .iter()
                        .any(|l| l.layer.name == *name)
                })
            }
            Self::AnimationPhase {
                actor,
                name,
                minimum,
                maximum,
            } => {
                actor.check(s)?;
                // Validate the requested interval even when the layer is absent.
                animation_phase_in_range(0., false, *minimum, *maximum)?;
                let mut reached = false;
                if let Some(resident) = s.simulation.characters.get(&actor.object) {
                    for layer in resident.character.skin.animator.layers() {
                        if layer.layer.name == *name {
                            reached |= animation_phase_in_range(
                                layer.layer.clock.phase,
                                layer.layer.clock.ended,
                                *minimum,
                                *maximum,
                            )?;
                        }
                    }
                }
                reached
            }
            Self::Level { actor, value } => {
                actor.check(s)?;
                s.simulation
                    .vm
                    .world
                    .positions
                    .get(&actor.object)
                    .context("milestone actor position absent")?
                    .level
                    == *value
            }
            Self::ScriptFrame {
                actor,
                source,
                bhav,
                present,
            } => {
                actor.check(s)?;
                script_frame_present(&s.simulation.vm, actor.object, source, *bhav)? == *present
            }
            Self::PersonField {
                actor,
                field,
                value,
            } => {
                actor.check(s)?;
                s.simulation.vm.world.objects[&actor.object]
                    .person
                    .as_ref()
                    .and_then(|p| p.get(field))
                    == Some(value)
            }
            Self::Contained { actor, target } => {
                actor.check(s)?;
                s.simulation.vm.world.objects[&actor.object]
                    .container
                    .is_some_and(|(id, _)| id == *target)
            }
            Self::ActionActive { actor, target } => {
                actor.check(s)?;
                s.simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&actor.object))
                    .and_then(|a| a.active.as_ref())
                    .is_some_and(|a| a.target == *target)
            }
            Self::TargetAbsent { actor, target } => {
                actor.check(s)?;
                s.simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&actor.object))
                    .is_some_and(|a| {
                        a.active
                            .iter()
                            .chain(a.queue.pending.iter())
                            .all(|a| a.target != *target)
                    })
            }
            Self::InteractionReleased { actor, target } => {
                actor.check(s)?;
                target.check(s)?;
                interaction_released(&s.simulation.vm, actor.object, target.object)?
            }
            Self::ObjectUseCount { target, count } => {
                target.check(s)?;
                ensure!(*count >= 0, "negative expected action-use count");
                object_use_count(&s.simulation.vm, target.object)? == *count
            }
            Self::ObjectUsesAccounted { target } => {
                target.check(s)?;
                object_uses_accounted(&s.simulation.vm, target.object)?
            }
            Self::SolePersonContained { actor, target } => {
                actor.check(s)?;
                target.check(s)?;
                sole_person_contained(&s.simulation.vm, actor.object, target.object)?
            }
            Self::PreviousActionReturn {
                actor,
                target,
                returned,
            } => {
                actor.check(s)?;
                target.check(s)?;
                action_owner(&s.simulation.vm, actor.object)?
                    .previous
                    .as_ref()
                    .is_some_and(|a| a.target == target.object && (a.flags & 32 != 0) == *returned)
            }
            Self::QueueEmpty { actor } => {
                actor.check(s)?;
                s.simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&actor.object))
                    .is_some_and(|a| a.active.is_none() && a.queue.pending.is_empty())
            }
            Self::MenuReady { minimum_actions } => app
                .menu()
                .is_some_and(|m| m.actions().len() >= *minimum_actions),
            Self::NoMenu => {
                s.host.presentation.object_input.is_none()
                    && s.host.presentation.ground_input.is_none()
            }
            Self::Modal {
                source,
                title,
                body_sha256,
            } => modal_matches(&s.simulation.vm, source, title, body_sha256),
            Self::Selected { actor } => {
                actor.check(s)?;
                s.simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    == Some(actor.object)
            }
            Self::Mode { value } => s
                .host
                .presentation
                .presenter
                .as_ref()
                .and_then(|p| p.camera.as_ref())
                .is_some_and(|c| c.settings.application_mode == *value),
            Self::FurnitureEdit { expected } => expected.matches(&buy_edit_cycle::capture(app)?),
            Self::All { conditions } => {
                ensure!(!conditions.is_empty(), "empty milestone conjunction");
                let mut reached = true;
                for condition in conditions {
                    reached &= condition.reached(app, start)?;
                }
                reached
            }
            Self::Any { conditions } => {
                ensure!(!conditions.is_empty(), "empty milestone disjunction");
                let mut reached = false;
                for condition in conditions {
                    // Validate every named lifetime even if an earlier branch
                    // matched; stale identities must never be hidden by OR.
                    reached |= condition.reached(app, start)?;
                }
                reached
            }
        })
    }
}

fn animation_phase_in_range(phase: f32, ended: bool, minimum: f32, maximum: f32) -> Result<bool> {
    ensure!(
        minimum.is_finite()
            && maximum.is_finite()
            && 0. <= minimum
            && minimum < maximum
            && maximum <= 1.,
        "invalid animation milestone phase interval"
    );
    ensure!(phase.is_finite(), "nonfinite retained animation phase");
    Ok(!ended && (minimum..maximum).contains(&phase))
}
fn script_frame_present(
    vm: &sims_vm::Vm,
    actor: i16,
    source: &str,
    bhav: Option<u16>,
) -> Result<bool> {
    ensure!(!source.is_empty(), "empty milestone script source");
    Ok(vm.threads.values().any(|thread| {
        thread.stack.iter().any(|frame| {
            frame.caller == actor
                && frame.location.source == source
                && bhav.is_none_or(|id| frame.location.bhav == id)
        })
    }))
}

fn action_owner(vm: &sims_vm::Vm, actor: i16) -> Result<&sims_vm::action::Actor> {
    vm.world
        .actions
        .as_ref()
        .and_then(|a| a.actors.get(&actor))
        .context("milestone actor action owner absent")
}
fn object_use_count(vm: &sims_vm::Vm, target: i16) -> Result<i16> {
    let count = vm
        .world
        .objects
        .get(&target)
        .and_then(|o| o.data.get(&62))
        .copied()
        .context("milestone target action-use count absent")?;
    ensure!(count >= 0, "negative retained action-use count");
    Ok(count)
}
fn object_uses_accounted(vm: &sims_vm::Vm, target: i16) -> Result<bool> {
    let records: usize = vm
        .threads
        .values()
        .map(|t| {
            t.stack
                .iter()
                .filter(|f| f.action_target == Some(target))
                .count()
                + t.completed_actions
                    .iter()
                    .filter(|(_, id, ..)| *id == target)
                    .count()
        })
        .sum();
    Ok(usize::try_from(object_use_count(vm, target)?)? == records)
}
fn interaction_released(vm: &sims_vm::Vm, actor: i16, target: i16) -> Result<bool> {
    let actions = action_owner(vm, actor)?;
    let object = vm
        .world
        .objects
        .get(&actor)
        .context("milestone actor absent")?;
    let target_object = vm
        .world
        .objects
        .get(&target)
        .context("milestone target absent")?;
    Ok(actions
        .active
        .iter()
        .chain(actions.queue.pending.iter())
        .all(|a| a.target != target)
        && !object.container.is_some_and(|(id, _)| id == target)
        && !target_object.slots.contains(&Some(actor))
        && vm.threads.values().all(|t| {
            !t.stack
                .iter()
                .any(|f| f.caller == actor && f.action_target == Some(target))
                && !t
                    .completed_actions
                    .iter()
                    .any(|(a, t, ..)| *a == actor && *t == target)
        }))
}
fn sole_person_contained(vm: &sims_vm::Vm, actor: i16, target: i16) -> Result<bool> {
    let person = vm
        .world
        .objects
        .get(&actor)
        .context("milestone actor absent")?;
    ensure!(person.person.is_some(), "milestone actor is not a person");
    let container = vm
        .world
        .objects
        .get(&target)
        .context("milestone container absent")?;
    Ok(person.container.is_some_and(|(id, slot)| {
        id == target && container.slots.get(usize::from(slot)) == Some(&Some(actor))
    }) && vm.world.objects.iter().all(|(&id, o)| {
        id == actor || o.person.is_none() || !o.container.is_some_and(|(id, _)| id == target)
    }))
}
fn modal_matches(vm: &sims_vm::Vm, source: &str, title: &str, body_sha256: &str) -> bool {
    let dialogs = &vm.world.dialogs;
    dialogs.records.iter().any(|r| {
        Some(r.id) == dialogs.modal
            && r.source == source
            && r.title == title
            && engine::digest(r.body.as_bytes()) == body_sha256
    })
}

#[cfg(test)]
#[path = "milestone_tests.rs"]
mod tests;
