//! Scoped, read-only capture support for synchronous simulation service cuts.
//! The stepping adapter, not a public caller, owns the observation callback.
#[cfg(test)]
use super::Saved;
use super::{Options, Session, View, managed};
use anyhow::{Result, ensure};
use sims_content::{Index, character_loading::live::Host, saves::House};
use sims_sim::Simulation;
use sims_vm::Program;
use std::sync::Arc;

#[derive(Clone, Copy)]
#[cfg(test)]
pub(super) enum Format {
    Ordinary,
    Managed,
}

struct Context<'session> {
    index: &'session Arc<Index>,
    managed_policy: managed::Policy,
    house: &'session House,
    program: &'session Program,
    host: &'session Host<View>,
    options: &'session Options,
}

impl Context<'_> {
    fn with_view<Output>(
        &self,
        simulation: &Simulation,
        observe: impl FnOnce(&Session) -> Result<Output>,
    ) -> Result<Output> {
        let assets = self.host.snapshot();
        let host = self.host.fork_for_exploration()?;
        ensure!(
            postcard::to_allocvec(&(&assets, &self.host.presentation))?
                == postcard::to_allocvec(&(host.snapshot(), &host.presentation))?,
            "service capture host fork changed saved ownership"
        );
        let observation = Session {
            loaded_input_origin: None,
            index: Arc::clone(self.index),
            managed_policy: self.managed_policy,
            house: self.house.clone(),
            program: self.program.clone(),
            simulation: simulation.clone(),
            host,
            options: self.options.clone(),
        };
        if let Some(parent) = &observation.host.presentation.application_parent {
            parent.validate(
                observation.host.presentation.clock.as_ref(),
                observation.simulation.vm.tick,
            )?;
        }
        if let Some(clock) = &observation.host.presentation.clock {
            clock.validate(&observation)?;
        }
        observe(&observation)
    }
}

/// Borrow the exact session context for one synchronous simulation call only.
/// The private callback accepts observations of that simulation's borrowed slot,
/// never a later-poll Session or a foreign Simulation. The full Session view
/// exists only during the observer and cannot be borrowed out of the callback.
/// The trusted stepping closure must not replace the simulation in its slot.
/// Derived host caches may be rebuilt in the local fork but are never serviced;
/// every saved host byte must remain identical. The immutable index borrow does
/// not pin filesystem contents: managed saves retain their existing source
/// reads and hash checks, including schema1 neighborhood import and schema2's
/// genuine mutable ownership requirements.
pub(super) fn with_view<Cut, Observation, Output>(
    session: &mut Session,
    observer: &mut dyn FnMut(Cut, &Session) -> Result<Observation>,
    step: impl FnOnce(
        &Program,
        &mut Simulation,
        &mut dyn FnMut(Cut, &Simulation) -> Result<Observation>,
    ) -> Result<Output>,
) -> Result<Output> {
    let Session {
        loaded_input_origin: _,
        index,
        managed_policy,
        house,
        program,
        simulation,
        host,
        options,
    } = session;
    let context = Context {
        index,
        managed_policy: *managed_policy,
        house,
        program,
        host,
        options,
    };
    let owner = std::ptr::from_ref(&*simulation);
    let mut capture = |cut, observed: &Simulation| {
        ensure!(
            std::ptr::eq(owner, observed),
            "service capture simulation does not belong to this synchronous call"
        );
        context.with_view(observed, |view| observer(cut, view))
    };
    step(program, simulation, &mut capture)
}

#[cfg(test)]
fn with_session<Output>(
    session: &mut Session,
    format: Format,
    step: impl FnOnce(
        &Program,
        &mut Simulation,
        &mut dyn FnMut(&Simulation) -> Result<Saved>,
    ) -> Result<Output>,
) -> Result<Output> {
    with_view(
        session,
        &mut |(), view| match format {
            Format::Ordinary => Ok(view.save()),
            Format::Managed => view.save_managed(),
        },
        |program, simulation, observe| {
            step(program, simulation, &mut |simulation| {
                observe((), simulation)
            })
        },
    )
}

#[cfg(test)]
mod tests;
