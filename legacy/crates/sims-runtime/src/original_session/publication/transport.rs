//! Full-frame 4ce44f children use the same recovered lot setters as the scoped
//! transport host. Keep C2 suspended until every real listener returns.
use super::*;
use sims_vm::scheduler::{clock, transport as lot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Child {
    request: world::Request,
    lot: Option<lot::Change>,
    speed: Option<clock::Speed>,
    broadcast: bool,
}
impl Child {
    pub(super) fn new(request: world::Request) -> Option<Self> {
        let (lot, speed) = match request {
            world::Request::Pause => (Some(lot::Change::new(lot::Command::Pause)), None),
            world::Request::Resume => (Some(lot::Change::new(lot::Command::Resume)), None),
            world::Request::SetSpeed(v) => (None, Some(clock::Speed::new(v))),
            _ => return None,
        };
        Some(Self {
            request,
            lot,
            speed,
            broadcast: false,
        })
    }
    pub(super) fn validate(&self, p: &State, s: &Session) -> Result<()> {
        ensure!(
            p.request() == Some(self.request)
                && !p.camera
                && !p.tools
                && p.person.is_none()
                && p.drawing.is_none()
                && !p.notice,
            "lot publication lost its world caller"
        );
        ensure!(
            matches!(
                (self.request, &self.lot, &self.speed),
                (
                    world::Request::Pause | world::Request::Resume,
                    Some(_),
                    None
                ) | (world::Request::SetSpeed(_), None, Some(_))
            ),
            "lot publication child differs"
        );
        if let Some(c) = &self.lot {
            c.validate().map_err(anyhow::Error::msg)?;
        }
        if let Some(c) = &self.speed {
            c.validate().map_err(anyhow::Error::msg)?;
        }
        if self.broadcast {
            ensure!(
                s.host
                    .presentation
                    .notifications
                    .as_ref()
                    .is_some_and(|n| n.dispatch.root_message() == Some((0xc2, 0))),
                "lot publication lost C2 owner"
            );
        } else {
            ensure!(
                !s.house_notification_pending(),
                "unowned lot publication notice"
            );
        }
        Ok(())
    }
}
impl Session {
    pub(super) fn advance_publication_transport(&mut self) -> Result<()> {
        let mut child = self
            .host
            .presentation
            .publication
            .as_mut()
            .unwrap()
            .transport
            .take()
            .unwrap();
        // Retain native writes and the exact continuation even if a listener
        // exposes an unsupported service. Never turn a failure into an ack.
        let result = self.advance_publication_transport_inner(&mut child);
        let request = child.request;
        self.host
            .presentation
            .publication
            .as_mut()
            .unwrap()
            .transport = Some(child);
        if result? {
            self.host
                .presentation
                .publication
                .as_mut()
                .unwrap()
                .transport = None;
            self.publication_reply(request)?;
        }
        Ok(())
    }
    fn advance_publication_transport_inner(&mut self, c: &mut Child) -> Result<bool> {
        let g = &mut self.simulation.vm.world.globals;
        let needs_notice = if let Some(change) = &mut c.lot {
            let timing = if let Some(clock) = &mut self.host.presentation.clock {
                &mut clock.fields.timing
            } else {
                self.host
                    .presentation
                    .transport
                    .as_mut()
                    .context("transport absent")?
                    .timing
                    .as_mut()
                    .context("lot timing absent")?
            };
            change
                .advance(g.get_mut(&17).context("lot pause absent")?, timing, 0)
                .map_err(anyhow::Error::msg)?
                .is_some()
        } else {
            let clock = self
                .host
                .presentation
                .clock
                .as_mut()
                .context("speed clock absent")?;
            let (mut current, mut saved) = (g[&16], g[&18]);
            let notice = c
                .speed
                .as_mut()
                .unwrap()
                .advance(&mut clock.fields, &mut current, &mut saved)
                .map_err(anyhow::Error::msg)?;
            g.insert(16, current);
            g.insert(18, saved);
            notice
        };
        if !needs_notice {
            return Ok(true);
        }
        if !c.broadcast {
            self.begin_clock_notice(0xc2, 0)?;
            c.broadcast = true;
        }
        if self.advance_clock_notice()? {
            if let Some(change) = &mut c.lot {
                change
                    .acknowledge(lot::Request::Broadcast)
                    .map_err(anyhow::Error::msg)?;
            } else {
                c.speed
                    .as_mut()
                    .unwrap()
                    .acknowledge()
                    .map_err(anyhow::Error::msg)?;
            }
            c.broadcast = false;
        }
        Ok(false)
    }
}
