//!Concrete486ada children. A frontend cannot acknowledge release or history.
use super::*;
use sims_sim::object_catalog::move_tool::commit::{Call, Reply, Request, View};

impl View for Live<'_> {
    fn underlying(&self, cursor: u32) -> Result<Option<u32>, String> {
        Ok(self.world.objects[&id(self.world, cursor)?]
            .cursor
            .as_ref()
            .ok_or("Move commit cursor absent")?
            .underlying)
    }
    fn facing(&self, furniture: u32) -> Result<i16, String> {
        self.world.objects[&id(self.world, furniture)?]
            .data
            .get(&1)
            .copied()
            .ok_or("Move commit facing absent".into())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Execution {
    call: Call,
}
impl Execution {
    pub(super) fn complete(&self) -> bool {
        self.call.outcome().is_some()
    }
    pub(super) fn outcome(&self) -> Option<bool> {
        self.call.outcome()
    }
    pub(super) fn awaiting_payload_clear(&self) -> bool {
        self.call.awaiting_payload_clear()
    }
    pub(super) fn owns_release(&self) -> bool {
        matches!(self.call.request(), Some(Request::ReleaseCursor(_)))
    }
    pub(super) fn validate(&self, s: &Session, state: &State) -> Result<()> {
        self.call.validate().map_err(anyhow::Error::msg)?;
        ensure!(state.preview_fields.is_some(), "Move commit fields absent");
        if let Some(Request::ReleaseCursor(h)) = self.call.request() {
            ensure!(
                state.tool.held == Some(h)
                    && s.simulation
                        .cursor_release
                        .as_ref()
                        .is_some_and(|r| r.cursor() == h),
                "Move commit lost its release child"
            );
        }
        if let Some(Request::Shared(request)) = self.call.request() {
            let dispatch = &state
                .call
                .as_ref()
                .context("Move commit caller absent")?
                .dispatch;
            //During cleanup the request may have concrete nested children.
            //Reconstruct the root identity rather than trusting a pending flag.
            ensure!(
                dispatch.complete() || dispatch.owns_direct(request),
                "Move commit shared child differs from its caller"
            );
        }
        Ok(())
    }
    pub(super) fn advance(&mut self, s: &mut Session, state: &mut State) -> Result<Advance> {
        let mut instructions = 0;
        match self.call.request() {
            Some(Request::ReleaseCursor(_)) => {
                instructions = s
                    .simulation
                    .advance_cursor_release(&s.program, 1)
                    .map_err(anyhow::Error::msg)?
                    .trace
                    .len();
                s.move_tool_assets()?;
                if s.simulation
                    .cursor_release
                    .as_ref()
                    .unwrap()
                    .outcome()
                    .is_some()
                {
                    let released = s
                        .simulation
                        .finish_cursor_release(&s.program)
                        .map_err(anyhow::Error::msg)?;
                    self.call
                        .acknowledge(self.call.request().unwrap(), Reply::Released(released))
                        .map_err(anyhow::Error::msg)?;
                }
            }
            Some(request @ Request::Shared(_)) => {
                //The outer driver only calls us after the concrete dispatch
                //and any history/notification child have returned.
                self.call
                    .acknowledge(request, Reply::Done)
                    .map_err(anyhow::Error::msg)?;
            }
            None => (),
        }
        if self.call.request().is_none() {
            let direction = &mut state
                .preview_fields
                .as_mut()
                .context("Move commit fields absent")?
                .direction;
            let live = Live {
                world: &s.simulation.vm.world,
                objects: &mut state.history.objects,
            };
            match self
                .call
                .advance(&mut state.tool, direction, &live)
                .map_err(anyhow::Error::msg)?
            {
                Some(Request::ReleaseCursor(h)) => {
                    s.simulation
                        .begin_cursor_release(
                            &s.program,
                            id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?,
                        )
                        .map_err(anyhow::Error::msg)?;
                }
                Some(Request::Shared(request)) => {
                    state.call.as_mut().unwrap().dispatch = match request {
                        cleanup::Request::Append { clipboard, payload } => {
                            dispatch::Dispatch::append(clipboard, payload)
                        }
                        cleanup::Request::ReleasePayload(payload) => {
                            dispatch::Dispatch::release_payload(payload)
                        }
                        cleanup::Request::ClearCatalogSelection => {
                            dispatch::Dispatch::cleanup(cleanup::Kind::ClearCatalogSelection)
                        }
                        cleanup::Request::Cursor => {
                            dispatch::Dispatch::cleanup(cleanup::Kind::Cursor)
                        }
                        _ => anyhow::bail!("unexpected Move commit child {request:?}"),
                    };
                }
                None => (),
            }
        }
        if state.tool.payload.is_none() {
            state.call.as_mut().unwrap().retired_payload = None;
        }
        Ok(Advance {
            progress: if self.complete() {
                Progress::Complete
            } else {
                Progress::Pending
            },
            instructions,
        })
    }
}
