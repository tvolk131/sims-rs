//! Retained native Session replay, shared by research tools and conformance.
//! No constructors, new clock inputs or substituted pending work are supplied.
use super::Session;
use anyhow::Result;
mod program;
pub use program::ProgramInputs;
pub mod trace;
pub mod retained;
pub fn require_immutable_index(index: &sims_content::Index) -> Result<()> {
    super::completed_baseline::immutable_index(index)
}
/// Research replay also supports an unbound scheduler. Native conformance uses
/// `advance_with_retained_clock` so caller loss cannot select that path.
pub fn advance(
    session: &mut Session,
    trace: &mut Vec<sims_vm::Trace>,
    observer: Option<&mut dyn FnMut(&Session) -> Result<()>>,
) -> Result<super::Progress> {
    if session.clock_pending() {
        return advance_with_retained_clock(session, trace, observer);
    }
    match observer {
        Some(observer) => session.step_with_dependent_observer(trace, observer),
        None => session.step_with_trace(trace),
    }
}

/// Advance a captured caller continuation. Conformance must not replace a
/// missing/completed caller with a direct scheduler step or a new clock frame.
/// Reject that condition before touching VM state, trace or the observer.
pub fn advance_with_retained_clock(
    session: &mut Session,
    trace: &mut Vec<sims_vm::Trace>,
    mut observer: Option<&mut dyn FnMut(&Session) -> Result<()>>,
) -> Result<super::Progress> {
    anyhow::ensure!(
        session.clock_pending(),
        "native conformance requires the retained elapsed caller continuation"
    );
    for _ in 0..4096 {
        let progress = match observer.as_mut() {
            Some(observer) => {
                session.advance_clock_with_dependent_observer(32, trace, &mut **observer)?
            }
            None => session.advance_clock_with_trace(32, trace)?,
        };
        match progress {
            super::clock::Progress::Household(progress) => return Ok(progress),
            super::clock::Progress::Pending => (),
            other => {
                anyhow::bail!("retained elapsed caller did not reach its next lot child: {other:?}")
            }
        }
    }
    anyhow::bail!("retained elapsed diagnostic work bound exceeded");
}
