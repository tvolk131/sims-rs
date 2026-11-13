use anyhow::{Context, Result, ensure};
use sims_vm::{ObjectId, World};

#[cfg(test)]
#[path = "resident_portrait_tests.rs"]
mod integration_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Target {
    pub point: [i32; 2],
    pub level: i32,
}

pub(super) fn target(world: &World, identity: ObjectId) -> Result<Option<Target>> {
    let object = world
        .objects
        .get(&identity)
        .context("portrait centering person absent")?;
    ensure!(
        object.person.is_some(),
        "portrait centering target is not a person"
    );
    if object.container.is_some() {
        return Ok(None);
    }
    if *object
        .data
        .get(&29)
        .context("portrait centering room absent")?
        < 0
    {
        return Ok(None);
    }
    let position = world
        .positions
        .get(&identity)
        .context("portrait centering position absent")?;
    Ok(Some(Target {
        point: [position.y, position.x],
        level: position.level,
    }))
}

impl super::Viewer {
    pub(super) fn advance_camera_motion_frame(&mut self) -> Result<bool> {
        if self.presentation_pending() || self.device.is_none() {
            return Ok(true);
        }
        let Some(session) = &mut self.session else {
            return Ok(true);
        };
        if !session.camera_motion_active()
            || session.player_input_pending()
            || session.simulation.vm.world.dialogs.modal.is_some()
        {
            return Ok(true);
        }
        let device = self.device.as_mut().unwrap();
        device.begin(
            session,
            sims_runtime::original_session::camera_command::Change::MotionFrame,
        )?;
        for _ in 0..32 {
            if device.advance(session, 256)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::{Object, autonomy::Placement};

    fn world() -> World {
        let mut world = World::default();
        world.objects.insert(
            134,
            Object {
                person: Some(Default::default()),
                data: [(2, 0), (29, 7)].into(),
                ..Default::default()
            },
        );
        world.positions.insert(
            134,
            Placement {
                x: 440,
                y: 616,
                level: 1,
            },
        );
        world
    }

    #[test]
    fn portrait_centering_uses_actual_native_position_order_without_mutation() -> Result<()> {
        let world = world();
        let before = serde_json::to_value(&world)?;
        assert_eq!(
            target(&world, 134)?,
            Some(Target {
                point: [616, 440],
                level: 1,
            })
        );
        assert_eq!(serde_json::to_value(&world)?, before);
        Ok(())
    }

    #[test]
    fn portrait_centering_rejects_contained_and_native_off_lot_people() -> Result<()> {
        let mut world = world();
        world.objects.get_mut(&134).unwrap().container = Some((164, 0));
        world.positions.remove(&134);
        assert_eq!(target(&world, 134)?, None);
        let object = world.objects.get_mut(&134).unwrap();
        object.container = None;
        object.data.insert(29, -1);
        assert_eq!(target(&world, 134)?, None);
        Ok(())
    }

    #[test]
    fn portrait_centering_rejects_missing_person_room_and_position() {
        let mut world = world();
        assert!(target(&world, 137).is_err());
        world.objects.get_mut(&134).unwrap().person = None;
        assert!(target(&world, 134).is_err());
        let object = world.objects.get_mut(&134).unwrap();
        object.person = Some(Default::default());
        object.data.remove(&29);
        assert!(target(&world, 134).is_err());
        world.objects.get_mut(&134).unwrap().data.insert(29, 0);
        world.positions.remove(&134);
        assert!(target(&world, 134).is_err());
    }
}
