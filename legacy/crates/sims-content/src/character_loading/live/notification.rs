//! Saved notification jobs bind the exact pending person Update to its presenter.
use super::*;
use crate::notifications::Software;
use sims_sim::{
    person_notification::{self as notify, display},
    person_update,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub parent: person_update::Request,
    pub tick: i32,
    dispatch: notify::Execution,
    display: Option<display::Execution>,
}
impl Job {
    pub fn validate(&self, id: ObjectId, presenter: &Software) -> Result<()> {
        ensure!(
            matches!(
                self.parent,
                person_update::Request::NotifyPerson { .. }
                    | person_update::Request::NotifyAnimationName { .. }
            ),
            "invalid notification parent"
        );
        self.dispatch.validate().map_err(anyhow::Error::msg)?;
        if let Some(child) = &self.display {
            let Some(notify::Request::Display { text, style }) = self.dispatch.pending_request()
            else {
                anyhow::bail!("notification display lost dispatch owner");
            };
            let colors = Software::colors(*style);
            ensure!(
                child.input() == (text.as_slice(), colors[0], colors[1]),
                "notification display inputs changed"
            );
            let cache = presenter
                .saved
                .entries
                .get(&id)
                .context("notification cache owner absent")?;
            child.validate(&cache.cache).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
}
impl<P: Presentation> Host<P> {
    pub fn pending_notifications(&self) -> &BTreeMap<ObjectId, Job> {
        &self.notification_jobs
    }

    pub(super) fn validate_notification_bindings(&self, simulation: &Simulation) -> Result<()> {
        // Native notification calls are synchronous and share a mutable font
        // color. Do not interleave two people between set-color and restore.
        ensure!(
            self.notification_jobs.len() <= 1,
            "overlapping notification font owners"
        );
        if let Some(p) = &self.notifications {
            p.validate()?;
            for (&id, entry) in &p.saved.entries {
                ensure!(
                    !entry.surface.as_ref().is_some_and(|s| s.locked)
                        || self
                            .notification_jobs
                            .get(&id)
                            .is_some_and(|j| j.display.is_some()),
                    "locked notification surface lost its continuation"
                );
            }
        }
        for (&id, resident) in &simulation.characters {
            let pass = resident.person_update.as_ref();
            ensure!(
                pass.is_some_and(|p| p.notification_started)
                    == self.notification_jobs.contains_key(&id),
                "person notification lost its presentation continuation"
            );
            if let Some(job) = self.notification_jobs.get(&id) {
                let pass = pass.unwrap();
                ensure!(
                    pass.execution.tick() == job.tick
                        && pass.execution.pending_request() == Some(&job.parent),
                    "person notification continuation has a stale owner"
                );
                job.validate(
                    id,
                    self.notifications
                        .as_ref()
                        .context("notification presenter absent")?,
                )?;
            }
        }
        ensure!(
            self.notification_jobs
                .keys()
                .all(|id| simulation.characters.contains_key(id)),
            "notification continuation outlived person"
        );
        Ok(())
    }

    /// A rendered frame supplies its actually visible people. Jobs remain
    /// synchronous with Update and cannot be discarded halfway through a draw.
    pub fn present_notifications(&mut self, now: u32, visible: &[ObjectId]) -> Result<()> {
        for id in visible {
            ensure!(
                !self.notification_jobs.contains_key(id),
                "presenting an unfinished notification"
            );
            if let Some(p) = &mut self.notifications {
                p.present(*id, now);
            }
        }
        Ok(())
    }

    pub(super) fn service_notifications(&mut self, simulation: &mut Simulation) -> Result<usize> {
        let Some(presenter) = &mut self.notifications else {
            return Ok(0);
        };
        let mut completed = 0;
        let active = self.notification_jobs.keys().next().copied();
        for (id, parent) in simulation.pending_person_services() {
            if active.is_some_and(|owner| owner != id) {
                continue;
            }
            let input = match &parent {
                person_update::Request::NotifyPerson { value } => notify::Input::Person(*value),
                person_update::Request::NotifyAnimationName { name } => {
                    notify::Input::AnimationName(name.clone())
                }
                _ => continue,
            };
            if !self.notification_jobs.contains_key(&id) {
                let dispatch = notify::Execution::new(input, notify::View::Active)
                    .map_err(anyhow::Error::msg)?;
                let tick = simulation
                    .begin_person_notification(id, &parent)
                    .map_err(anyhow::Error::msg)?;
                self.notification_jobs.insert(
                    id,
                    Job {
                        parent: parent.clone(),
                        tick,
                        dispatch,
                        display: None,
                    },
                );
            }
            let job = self.notification_jobs.get_mut(&id).unwrap();
            match job.dispatch.next() {
                Some(notify::Request::FormatMoney { amount }) => {
                    job.dispatch
                        .acknowledge(notify::Response::Text(
                            presenter.saved.locale.format(amount),
                        ))
                        .map_err(anyhow::Error::msg)?;
                }
                Some(notify::Request::Sound { name }) => {
                    // Same original sound entry/actor identity as animation
                    // sound services, without the animation visibility gate.
                    let response = self.presentation.property(
                        id,
                        &properties::Request::Sound {
                            name: String::from_utf8(name)?,
                            object: id,
                        },
                    )?;
                    ensure!(
                        matches!(response, Response::Done),
                        "unexpected notification sound response"
                    );
                    job.dispatch
                        .acknowledge(notify::Response::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                Some(notify::Request::Display { text, style }) => {
                    if job.display.is_none() {
                        let colors = Software::colors(style);
                        job.display = Some(
                            display::Execution::new(text, colors[0], colors[1], true)
                                .map_err(anyhow::Error::msg)?,
                        );
                        presenter.saved.entries.entry(id).or_default();
                    }
                    let child = job.display.as_mut().unwrap();
                    let request = child
                        .next(&mut presenter.saved.entries.get_mut(&id).unwrap().cache)
                        .map_err(anyhow::Error::msg)?;
                    if let Some(request) = request {
                        let response =
                            presenter.service(id, &request, simulation.vm.tick as u32)?;
                        child
                            .acknowledge(
                                &mut presenter.saved.entries.get_mut(&id).unwrap().cache,
                                response,
                            )
                            .map_err(anyhow::Error::msg)?;
                    } else {
                        ensure!(child.is_complete(), "notification display incomplete");
                        job.display = None;
                        job.dispatch
                            .acknowledge(notify::Response::Done)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                None => {
                    ensure!(
                        job.dispatch.is_complete() && job.display.is_none(),
                        "notification unfinished"
                    );
                    simulation
                        .complete_person_notification(id, &parent)
                        .map_err(anyhow::Error::msg)?;
                    self.notification_jobs.remove(&id);
                }
            }
            completed += 1;
            break;
        }
        Ok(completed)
    }
}
