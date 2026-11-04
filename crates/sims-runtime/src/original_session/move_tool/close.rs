//!4933dc clears undo before redo; each485a19 release runs its existing concrete
//!command destructor only when the reference count reaches zero.
use super::{cleanup, history};
use crate::original_session::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub enum Progress {
    Pending,
    Complete,
    Service(super::Service),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Close {
    manager: u32,
    phase: u8,
    child: Option<Box<history::Execution>>,
}
impl Close {
    pub fn new(manager: u32) -> Self {
        Self {
            manager,
            phase: 0,
            child: None,
        }
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        let moving = s
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("history close lost Move owner")?;
        ensure!(
            self.manager != 0 && moving.tool.clipboard == self.manager && self.phase <= 2,
            "history close manager or phase differs"
        );
        if let Some(child) = &self.child {
            let expected = match self.phase {
                0 => cleanup::Request::ClearUndo(self.manager),
                1 => cleanup::Request::ClearRedo(self.manager),
                _ => anyhow::bail!("completed history close retained a child"),
            };
            ensure!(child.request() == expected, "history close child differs");
            child.validate(s, &moving.history, moving.inputs.registry)?;
        }
        Ok(())
    }
    pub fn advance(&mut self, s: &mut Session) -> Result<Progress> {
        self.validate(s)?;
        if self.phase == 2 {
            return Ok(Progress::Complete);
        }
        if self.child.is_none() {
            let r = if self.phase == 0 {
                cleanup::Request::ClearUndo(self.manager)
            } else {
                cleanup::Request::ClearRedo(self.manager)
            };
            self.child = Some(Box::new(
                history::Execution::new(r)?.context("history clear child absent")?,
            ));
        }
        let mut moving = s
            .host
            .presentation
            .move_tool
            .take()
            .context("history close Move owner absent")?;
        let result =
            self.child
                .as_mut()
                .unwrap()
                .advance(s, &mut moving.history, moving.inputs.registry);
        s.host.presentation.move_tool = Some(moving);
        match result? {
            history::Step::Complete => {
                self.child = None;
                self.phase += 1;
                Ok(Progress::Pending)
            }
            history::Step::Pending(_) => Ok(Progress::Pending),
            history::Step::Service(service) => Ok(Progress::Service(service)),
        }
    }
}
