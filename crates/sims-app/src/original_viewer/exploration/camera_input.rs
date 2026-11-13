use super::*;
use iced::{Event, Point, Rectangle, mouse};

fn emit(
    pointer: &mut viewport_drag::Pointer,
    size: [u32; 2],
    pixel: [u32; 2],
    down: Option<bool>,
) -> Result<Message> {
    ensure!(
        !size.contains(&0) && pixel.iter().zip(size).all(|(&axis, limit)| axis < limit),
        "camera pointer is outside the current scene"
    );
    let position = Point::new(pixel[0] as f32, pixel[1] as f32);
    let cursor = mouse::Cursor::Available(position);
    let bounds = Rectangle {
        x: 0.,
        y: 0.,
        width: size[0] as f32,
        height: size[1] as f32,
    };
    let event = Event::Mouse(match down {
        Some(true) => mouse::Event::ButtonPressed(mouse::Button::Right),
        Some(false) => mouse::Event::ButtonReleased(mouse::Button::Right),
        None => mouse::Event::CursorMoved { position },
    });
    if down.is_some() {
        pointer.right_button(&event, size, bounds, cursor)
    } else {
        pointer.motion(&event, size, bounds, cursor)
    }
    .context("camera pointer event has no active input owner")
}

impl Application {
    pub(super) fn camera_input(&mut self, pixel: [u32; 2], down: Option<bool>) -> Result<()> {
        ensure!(
            !self.viewer.current_move() && !self.viewer.viewport_drag_blocked(),
            "camera pointer is blocked by another input or presentation owner"
        );
        let scene = self.viewer.scene.as_ref().context("camera scene absent")?;
        let message = emit(
            &mut self.camera_pointer,
            [scene.width, scene.height],
            pixel,
            down,
        )?;
        self.dispatch(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_motion_uses_the_actual_edge_pan_mapping() -> Result<()> {
        let mut pointer = viewport_drag::Pointer::default();
        for (pixel, expected) in [
            ([1, 450], [1, 0]),
            ([1319, 450], [-1, 0]),
            ([660, 1], [0, 1]),
            ([660, 899], [0, -1]),
            ([660, 450], [0, 0]),
        ] {
            let message = emit(&mut pointer, [1320, 900], pixel, None)?;
            ensure!(
                matches!(message, Message::ViewportMotion { pixel: Some(actual), pan }
                if actual == pixel.map(|axis| axis as f32) && pan == expected)
            );
        }
        Ok(())
    }

    #[test]
    fn camera_right_hold_and_release_survive_pointer_serialization() -> Result<()> {
        let mut pointer = viewport_drag::Pointer::default();
        let pressed = emit(&mut pointer, [1320, 900], [400, 500], Some(true))?;
        ensure!(matches!(
            pressed,
            Message::ViewportRightButton {
                down: true,
                pixel: Some([400., 500.])
            }
        ));
        let bytes = serde_json::to_vec(&pointer)?;
        let mut restored: viewport_drag::Pointer = serde_json::from_slice(&bytes)?;
        ensure!(serde_json::to_vec(&restored)? == bytes);
        let moved = emit(&mut restored, [1320, 900], [1, 450], None)?;
        ensure!(matches!(
            moved,
            Message::ViewportMotion {
                pixel: Some([1., 450.]),
                pan: [0, 0]
            }
        ));
        let released = emit(&mut restored, [1320, 900], [432, 516], Some(false))?;
        ensure!(matches!(
            released,
            Message::ViewportRightButton {
                down: false,
                pixel: Some([432., 516.])
            }
        ));
        ensure!(emit(&mut restored, [1320, 900], [432, 516], Some(false)).is_err());
        let moved = emit(&mut restored, [1320, 900], [1, 450], None)?;
        ensure!(matches!(moved, Message::ViewportMotion { pan: [1, 0], .. }));
        Ok(())
    }

    #[test]
    fn retained_camera_drag_keeps_unapplied_displacement_and_release() -> Result<()> {
        let mut drag = viewport_drag::Drag::default();
        drag.begin(Some([400., 500.]), true);
        drag.motion(Some([412.25, 504.5]));
        ensure!(drag.take_scroll() == Some([12, 5]));
        drag.motion(Some([432.5, 516.25]));
        let bytes = serde_json::to_vec(&drag)?;
        let mut restored: viewport_drag::Drag = serde_json::from_slice(&bytes)?;
        ensure!(serde_json::to_vec(&restored)? == bytes);
        ensure!(restored.suppresses_pan());
        ensure!(restored.take_scroll() == drag.take_scroll());
        restored.end(Some([440., 520.]));
        drag.end(Some([440., 520.]));
        ensure!(restored.take_scroll() == drag.take_scroll());
        ensure!(!restored.suppresses_pan());
        Ok(())
    }

    #[test]
    fn camera_pointer_rejects_outside_scene_without_changing_held_input() -> Result<()> {
        let mut pointer = viewport_drag::Pointer::default();
        emit(&mut pointer, [1320, 900], [400, 500], Some(true))?;
        let bytes = serde_json::to_vec(&pointer)?;
        for size in [[0, 900], [1320, 0], [1320, 900]] {
            ensure!(emit(&mut pointer, size, [1320, 900], Some(false)).is_err());
            ensure!(serde_json::to_vec(&pointer)? == bytes);
        }
        ensure!(emit(&mut pointer, [1320, 900], [400, 500], Some(false)).is_ok());
        Ok(())
    }
}
