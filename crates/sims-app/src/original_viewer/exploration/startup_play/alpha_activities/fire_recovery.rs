use super::*;
use sims_vm::action::Action;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CookingFireRecovery {
    failed: FailedMeal,
    extinguish: Extinguish,
    retry: Activity,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct FailedMeal {
    activity: Activity,
    pub(super) food: Identity,
    serial: u32,
    thread: sims_vm::ThreadId,
    invocation: u64,
    tick: u64,
}

impl FailedMeal {
    pub(super) fn matches(&self, event: &ActionTerminalEvent) -> bool {
        event.actor == self.activity.actor.object
            && event.target == self.food.object
            && event.serial == self.serial
            && event.thread == Some(self.thread)
            && event.invocation == Some(self.invocation)
            && event.tick == self.tick
            && event.reason == ActionTerminalReason::NormalReturn
            && !event.cancelled
            && event.result == Some(false)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Extinguish {
    actor: Identity,
    fire: Identity,
    source: String,
    source_sha256: String,
    offered: Action,
    prior_active: Action,
    max_ticks: u64,
}

impl CookingFireRecovery {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.failed.activity.kind == Kind::Meal
                && self.retry.kind == Kind::Meal
                && self.failed.activity.resume.is_none()
                && self.retry.resume.is_none()
                && self.failed.activity.actor == self.retry.actor
                && self.failed.activity.actor == self.extinguish.actor
                && self.failed.activity.target == self.retry.target
                && self.failed.activity.routine == self.retry.routine
                && self.failed.serial != 0
                && self.failed.invocation != 0
                && self.failed.tick != 0,
            "cooking-fire recovery requires an exact failed meal and same-actor full retry"
        );
        ensure!(
            (1..=6000).contains(&self.extinguish.max_ticks)
                && self.extinguish.prior_active.routine == 4115
                && self.extinguish.prior_active.target == self.extinguish.fire.object
                && self.extinguish.prior_active.priority == 100
                && self.extinguish.prior_active.id != 0
                && self.extinguish.offered.routine == 4107
                && self.extinguish.offered.target == self.extinguish.fire.object
                && self.extinguish.offered.icon_owner == self.extinguish.fire.object
                && self.extinguish.offered.flags & 16 == 0,
            "cooking-fire recovery lacks pinned Panic cancellation and enabled Extinguish"
        );
        Ok(())
    }
}

pub(super) fn run(play: &mut Play<'_>, spec: &CookingFireRecovery, index: usize) -> Result<()> {
    spec.validate()?;
    ensure!(
        play.shadow.is_none(),
        "fire recovery overlaps restored-continuation comparison"
    );
    activity(play, &spec.failed.activity, index, Some(&spec.failed))?;
    extinguish(play, &spec.extinguish)?;
    activity(play, &spec.retry, index, None)?;
    play.progress.evidence.push(json!({"kind":"cooking_fire_recovery_completed",
        "step":index,"tick":play.tick()?,"failed_attempt_meal_credit":false,"successful_retry_meal_credit":true}));
    Ok(())
}

fn extinguish(play: &mut Play<'_>, spec: &Extinguish) -> Result<()> {
    ensure!(
        play.shadow.is_none(),
        "Extinguish overlaps restored-continuation comparison"
    );
    spec.actor.check(play.session()?)?;
    spec.fire.check(play.session()?)?;
    let session = play.session()?;
    let vm = &session.simulation.vm;
    let definition = vm.world.objects[&spec.fire.object]
        .definition
        .as_ref()
        .context("fire definition absent")?;
    ensure!(
        definition.guid == 617176985
            && definition.source == spec.source
            && session
                .program
                .modules
                .get(&spec.source)
                .is_some_and(|module| module.content_hash == spec.source_sha256)
            && vm.world.fires.registered == [spec.fire.object],
        "pinned fire source/registry changed"
    );
    ensure!(
        vm.world
            .actions
            .as_ref()
            .and_then(|state| state.actors.get(&spec.actor.object))
            .and_then(|owner| owner.active.as_ref())
            == Some(&spec.prior_active),
        "pinned prior Panic action changed"
    );
    play.execute(Command::Resident {
        identity: spec.actor.clone(),
    })?;
    play.execute(Command::Object {
        identity: spec.fire.clone(),
    })?;
    let offers = play
        .app
        .menu()
        .context("fire menu absent")?
        .actions()
        .to_vec();
    let choices: Vec<_> = offers
        .iter()
        .enumerate()
        .filter(|(_, action)| **action == spec.offered)
        .map(|(index, _)| index)
        .collect();
    let source_menu = play
        .session()?
        .simulation
        .vm
        .action_menu(&play.session()?.program, spec.fire.object)
        .map_err(anyhow::Error::msg)?
        .to_vec();
    ensure!(
        choices.len() == 1
            && source_menu
                .iter()
                .any(|entry| entry.id == spec.offered.menu_id
                    && entry.action == spec.offered.routine
                    && entry.label.eq_ignore_ascii_case("Extinguish")),
        "actual fire menu lacks the exact pinned enabled Extinguish offer"
    );
    play.progress.evidence.push(
        json!({"kind":"actual_fire_menu","actor":spec.actor,"fire":spec.fire,
        "offered_actions":offers,"source_menu":source_menu,"tick":play.tick()?}),
    );
    let previous = action_ids(&play.app, spec.actor.object)?;
    play.execute(Command::Choice {
        index: choices[0],
        expected: spec.offered.clone(),
    })?;
    let serials: Vec<_> = action_ids(&play.app, spec.actor.object)?
        .into_iter()
        .filter(|serial| !previous.contains(serial))
        .collect();
    ensure!(
        serials.len() == 1,
        "Extinguish did not enqueue one new action"
    );
    let serial = serials[0];
    play.progress.evidence.push(json!({"kind":"cancel_prior_active_for_extinguish",
        "actor":spec.actor,"action":spec.prior_active,"queued_extinguish_serial":serial,"tick":play.tick()?}));
    play.execute(Command::CancelAction {
        actor: spec.actor.clone(),
        serial: spec.prior_active.id,
    })?;
    let start = play.tick()?;
    let mut returned = false;
    let mut cancelled = false;
    play.execute(Command::Playing { value: true })?;
    loop {
        ensure!(
            play.tick()? - start < spec.max_ticks,
            "fire resolution tick limit"
        );
        let frame = play.frame()?;
        play.progress.evidence.push(json!({"kind":"fire_resolution_frame","tick":play.tick()?,
            "registered":play.session()?.simulation.vm.world.fires.registered,"terminals":frame.terminals}));
        for terminal in &frame.terminals {
            if terminal.actor != spec.actor.object {
                continue;
            }
            if terminal.serial == serial {
                ensure!(
                    terminal.target == spec.fire.object
                        && terminal.reason == ActionTerminalReason::NormalReturn
                        && terminal.result == Some(true)
                        && !terminal.cancelled
                        && terminal.invocation.is_some(),
                    "Extinguish lacks a successful qualified normal return"
                );
                returned = true;
            }
            if terminal.serial == spec.prior_active.id {
                ensure!(
                    terminal.target == spec.fire.object
                        && terminal.reason == ActionTerminalReason::NormalReturn
                        && terminal.cancelled
                        && terminal.result.is_some()
                        && terminal.invocation.is_some(),
                    "prior Panic lacks a qualified cancelled normal unwind"
                );
                cancelled = true;
            }
        }
        spec.actor.check(play.session()?)?;
        let vm = &play.session()?.simulation.vm;
        ensure!(
            vm.world
                .fires
                .registered
                .iter()
                .all(|object| *object == spec.fire.object),
            "fire spread during recovery"
        );
        if vm.world.objects.contains_key(&spec.fire.object) {
            spec.fire.check(play.session()?)?;
        }
        if returned
            && cancelled
            && vm.world.fires.registered.is_empty()
            && !vm.world.objects.contains_key(&spec.fire.object)
            && released_target(vm, spec.actor.object, spec.fire.object)?
        {
            play.progress.evidence.push(json!({"kind":"fire_resolved","actor":spec.actor,
                "fire":spec.fire,"serial":serial,"tick":play.tick()?,"released":true,"prior_panic_cancelled":true}));
            return play.execute(Command::Playing { value: false });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> FailedMeal {
        FailedMeal {
            activity: Activity {
                actor: Identity {
                    object: 18,
                    handle: 66,
                },
                kind: Kind::Meal,
                target: Identity {
                    object: 118,
                    handle: 26,
                },
                routine: 4098,
                max_ticks: 6000,
                resume: None,
            },
            food: Identity {
                object: 29,
                handle: 554,
            },
            serial: 85,
            thread: 66,
            invocation: 78,
            tick: 134456,
        }
    }

    #[test]
    fn expected_failed_meal_rejects_unqualified_cancelled_or_wrong_terminal() {
        let failed = fixture();
        let actual = ActionTerminalEvent {
            actor: 18,
            serial: 85,
            target: 29,
            thread: Some(66),
            invocation: Some(78),
            tick: 134456,
            reason: ActionTerminalReason::NormalReturn,
            cancelled: false,
            result: Some(false),
        };
        assert!(failed.matches(&actual));
        for mutate in [
            (|event: &mut ActionTerminalEvent| event.actor = 21) as fn(&mut ActionTerminalEvent),
            |event| event.serial += 1,
            |event| event.target += 1,
            |event| event.thread = None,
            |event| event.invocation = None,
            |event| event.tick += 1,
            |event| event.reason = ActionTerminalReason::Reset,
            |event| event.cancelled = true,
            |event| event.result = Some(true),
            |event| event.result = None,
        ] {
            let mut rejected = actual.clone();
            mutate(&mut rejected);
            assert!(!failed.matches(&rejected), "{rejected:?}");
        }
    }

    #[test]
    fn cooking_fire_recovery_requires_full_retry_and_rejects_unfrozen_fields() -> Result<()> {
        let failed = fixture();
        let offered = Action {
            target: 320,
            icon_owner: 320,
            routine: 4107,
            flags: 12,
            priority: 50,
            ..Action::default()
        };
        let prior_active = Action {
            id: 89,
            target: 320,
            icon_owner: 320,
            routine: 4115,
            flags: 29,
            priority: 100,
            ..Action::default()
        };
        let spec = CookingFireRecovery {
            retry: failed.activity.clone(),
            failed,
            extinguish: Extinguish {
                actor: Identity {
                    object: 18,
                    handle: 66,
                },
                fire: Identity {
                    object: 320,
                    handle: 585,
                },
                source: "original::Fire.iff".into(),
                source_sha256: "pinned".into(),
                offered,
                prior_active,
                max_ticks: 6000,
            },
        };
        spec.validate()?;
        let mut wrong = spec.clone();
        wrong.retry.kind = Kind::Hygiene;
        assert!(wrong.validate().is_err());
        let mut wrong = spec.clone();
        wrong.retry.actor.object = 21;
        assert!(wrong.validate().is_err());
        let mut wrong = spec.clone();
        wrong.extinguish.prior_active.routine = 4106;
        assert!(wrong.validate().is_err());
        let mut wrong = spec.clone();
        wrong.extinguish.offered.flags |= 16;
        assert!(wrong.validate().is_err());
        let mut serialized = serde_json::to_value(&spec)?;
        serialized["credit_failed_meal"] = json!(true);
        assert!(serde_json::from_value::<CookingFireRecovery>(serialized).is_err());
        Ok(())
    }
}
