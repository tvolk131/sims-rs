use super::{Device, Frame};
use anyhow::{Result, ensure};
use sims_runtime::original_session::Session;
use sims_sim::lot_presenter::{camera, motion::frame};

impl Device {
    pub(super) fn motion_frame_service(
        &mut self,
        session: &Session,
        request: &camera::Service,
    ) -> Result<camera::Reply> {
        use frame::displacement::Request as Displacement;
        match request {
            camera::Service::MotionFrame(frame::Request::Synchronize)
            | camera::Service::Displacement(Displacement::Synchronize) => {
                self.prepare(session)?;
            }
            camera::Service::Displacement(Displacement::FlushSurface) => {
                self.prepare(session)?;
                self.publish();
            }
            camera::Service::Displacement(Displacement::NotifyScroll) => (),
            camera::Service::Displacement(Displacement::DrawingEnabled) => {
                return Ok(camera::Reply::Enabled(0));
            }
            camera::Service::Displacement(
                Displacement::SetDrawMode {
                    first: 0,
                    mask: 15,
                    enabled: 0,
                }
                | Displacement::SetDrawMode {
                    first: 0,
                    mask: 8,
                    enabled: 1,
                },
            ) => {
                self.saved.dirty = true;
            }
            camera::Service::Displacement(Displacement::ScrollSurface {
                viewport,
                include_background,
            }) => {
                let camera = session
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .and_then(|presenter| presenter.camera.as_ref())
                    .ok_or_else(|| anyhow::anyhow!("camera displacement owner absent"))?;
                ensure!(
                    *viewport == camera.settings.viewport
                        && *include_background == u32::from(camera.settings.camera_mode == 0),
                    "camera displacement renderer inputs differ"
                );
                ensure!(
                    Frame::current(session)?.input.viewport[2..]
                        == [
                            viewport[2].wrapping_sub(viewport[0]) as u32,
                            viewport[3].wrapping_sub(viewport[1]) as u32,
                        ],
                    "camera displacement surface dimensions differ"
                );
                self.prepare(session)?;
                self.publish();
            }
            camera::Service::Displacement(Displacement::Invalidate) => {
                self.prepare(session)?;
                self.publish();
            }
            _ => anyhow::bail!("camera motion needs its original owner: {request:?}"),
        }
        Ok(camera::Reply::Done)
    }
}
