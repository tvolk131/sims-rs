use super::{Device, Frame, Saved};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_runtime::original_session::{Session, presenter::Progress};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Ownership {
    entry_tick: u64,
    frame: Frame,
    published_frame: Frame,
    dirty: bool,
    generation: u64,
    published_generation: u64,
}

impl Ownership {
    fn capture(saved: &Saved, entry_tick: u64) -> Self {
        Self {
            entry_tick,
            frame: saved.frame,
            published_frame: saved.published_frame,
            dirty: saved.dirty,
            generation: saved.generation,
            published_generation: saved.published_generation,
        }
    }

    fn validate(self, saved: &Saved, tick: u64) -> Result<()> {
        ensure!(
            self == Self::capture(saved, tick)
                && self.frame.tick <= tick
                && self.published_frame.tick == self.frame.tick,
            "normal-frame published surface recipe changed before publication"
        );
        Ok(())
    }

    fn require_publication(
        self,
        saved: &Saved,
        tick: u64,
        native_pending: bool,
        view_pending: bool,
    ) -> Result<()> {
        self.validate(saved, tick)?;
        ensure!(
            !native_pending && !view_pending,
            "native normal-frame caller still owns the surface"
        );
        Ok(())
    }
}

pub(super) fn validate_tick(saved: &Saved, tick: u64) -> Result<()> {
    if let Some(owner) = saved.normal_frame {
        owner.validate(saved, tick)?;
    } else {
        ensure!(
            saved.frame.tick == tick,
            "camera surface belongs to another simulation tick"
        );
    }
    ensure!(
        saved.published_frame.tick == saved.frame.tick,
        "camera published surface belongs to another tick"
    );
    Ok(())
}

pub(super) fn validate(saved: &Saved, session: &Session) -> Result<()> {
    ensure!(
        !session.normal_view_frame_pending() || saved.normal_frame.is_some(),
        "native normal-frame lost its renderer surface owner"
    );
    if let Some(owner) = saved.normal_frame {
        owner.validate(saved, session.simulation.vm.tick)?;
        let presenter = session
            .host
            .presentation
            .presenter
            .as_ref()
            .context("normal-frame surface lost its native presenter")?;
        presenter
            .validate_simulation(&session.simulation)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            session.normal_view_frame_pending() || presenter.complete(),
            "another view caller took the normal-frame surface"
        );
    }
    Ok(())
}

pub(super) fn validate_pixels(saved: &Saved) -> Result<()> {
    if saved.normal_frame.is_some() {
        let pixels = saved
            .frozen_pixels
            .as_ref()
            .context("normal-frame surface lost its exact published pixels")?;
        let [_, _, width, height] = saved.published_frame.input.viewport;
        ensure!(
            width > 0
                && height > 0
                && (width as usize)
                    .checked_mul(height as usize)
                    .and_then(|area| area.checked_mul(4))
                    == Some(pixels.len()),
            "normal-frame published pixel dimensions differ"
        );
    }
    Ok(())
}

impl Device {
    pub fn normal_view_frame_pending(&self) -> bool {
        self.saved.normal_frame.is_some()
    }

    pub fn begin_normal_view_frame(&mut self, session: &mut Session) -> Result<()> {
        self.require_unowned_normal_frame(session)?;
        let owner = Ownership::capture(&self.saved, session.simulation.vm.tick);
        owner.validate(&self.saved, session.simulation.vm.tick)?;
        session.begin_normal_view_frame()?;
        self.saved.normal_frame = Some(owner);
        self.frozen = true;
        Ok(())
    }

    pub fn advance_normal_view_frame(
        &mut self,
        session: &mut Session,
        budget: usize,
    ) -> Result<Progress> {
        validate(&self.saved, session)?;
        ensure!(
            self.normal_view_frame_pending() && session.normal_view_frame_pending(),
            "normal-frame advance lost its retained caller"
        );
        session.advance_view_refresh(session.retained_camera_input()?, budget)
    }

    pub fn prepare_view_frame(&mut self, session: &mut Session) -> Result<()> {
        if session.simulation.vm.world.registry_order.is_none() {
            self.require_unowned_normal_frame(session)?;
            return session.prepare_view_frame();
        }
        if !self.normal_view_frame_pending() {
            self.begin_normal_view_frame(session)?;
        }
        validate(&self.saved, session)?;
        if !session.normal_view_frame_pending() {
            return Ok(());
        }
        for _ in 0..30000 {
            match self.advance_normal_view_frame(session, 256)? {
                Progress::Complete => return Ok(()),
                Progress::Pending => (),
                progress => {
                    anyhow::bail!("normal-frame requires an external view service: {progress:?}")
                }
            }
        }
        anyhow::bail!("normal-frame classification budget exceeded")
    }

    pub(super) fn require_unowned_normal_frame(&self, session: &Session) -> Result<()> {
        ensure!(
            !self.normal_view_frame_pending() && !session.normal_view_frame_pending(),
            "normal-frame owns the published surface"
        );
        Ok(())
    }

    pub(super) fn finish_normal_view_frame(&mut self, session: &Session) -> Result<()> {
        validate(&self.saved, session)?;
        let owner = self
            .saved
            .normal_frame
            .context("normal-frame surface already published")?;
        owner.require_publication(
            &self.saved,
            session.simulation.vm.tick,
            session.normal_view_frame_pending(),
            session.view_refresh_pending(),
        )?;
        ensure!(
            !session.player_input_blocks_ticks()
                && !session.simulation.selection_work_pending()
                && !session.house_notification_pending()
                && !session.ui_wait_pending()
                && !session.clock_pending()
                && !session.postload_pending()
                && !session.load_completion_pending()
                && !session.host.ordinary.updates_pending()
                && session.simulation.vm.scheduled_object_visit.is_none(),
            "suspended caller owns normal-frame surface publication"
        );
        let (frame, scene) = self.build_surface(session, false)?;
        self.publish_normal_frame_surface(
            frame,
            scene,
            session.simulation.vm.tick,
            session.normal_view_frame_pending(),
            session.view_refresh_pending(),
        )
    }

    fn publish_normal_frame_surface(
        &mut self,
        frame: Frame,
        scene: Arc<crate::Scene>,
        tick: u64,
        native_pending: bool,
        view_pending: bool,
    ) -> Result<()> {
        self.saved
            .normal_frame
            .context("normal-frame surface already published")?
            .require_publication(&self.saved, tick, native_pending, view_pending)?;
        ensure!(
            frame.tick == tick,
            "normal-frame replacement surface belongs to another simulation tick"
        );
        self.install_surface(frame, scene)?;
        self.publish();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
