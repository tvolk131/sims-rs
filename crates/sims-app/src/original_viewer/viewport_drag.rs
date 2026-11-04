use super::{Message, Viewer};
use iced::{Event, Rectangle, mouse};

#[derive(Default)]
pub(super) struct Drag {
    origin: Option<[f64; 2]>,
    endpoint: [f64; 2],
    intent: [f64; 2],
    camera_owned: bool,
    revision: u64,
}

impl Drag {
    pub(super) fn reset(&mut self) {
        *self = Self {
            revision: self.revision,
            ..Self::default()
        };
    }

    fn observe(&mut self, revision: u64, blocked: bool) {
        if self.revision != revision || blocked {
            self.reset();
            self.revision = revision;
        }
    }

    fn prepare(&mut self, message: &Message) {
        if !matches!(
            message,
            Message::Frame(_)
                | Message::ViewportEnter
                | Message::ViewportMotion { .. }
                | Message::ViewportRightButton { .. }
                | Message::Shift(_)
        ) {
            self.reset();
        }
    }

    pub(super) fn begin(&mut self, pixel: Option<[f32; 2]>, ready: bool) {
        let Some(pixel) = pixel.filter(|pixel| ready && pixel.iter().all(|axis| axis.is_finite()))
        else {
            self.reset();
            return;
        };
        if self.origin.is_some() {
            self.reset();
        }
        self.origin = Some(pixel.map(f64::from));
        self.endpoint = pixel.map(f64::from);
    }

    pub(super) fn motion(&mut self, pixel: Option<[f32; 2]>) {
        if self.origin.is_none() {
            return;
        }
        if let Some(pixel) = pixel.filter(|pixel| pixel.iter().all(|axis| axis.is_finite())) {
            self.endpoint = pixel.map(f64::from);
        } else {
            self.reset();
        }
    }

    pub(super) fn end(&mut self, pixel: Option<[f32; 2]>) {
        self.motion(pixel);
        self.intent = self.total();
        self.origin = None;
    }

    fn total(&self) -> [f64; 2] {
        [0, 1].map(|axis| {
            let displacement = self
                .origin
                .map_or(0., |origin| self.endpoint[axis] - origin[axis]);
            (self.intent[axis] + displacement).clamp(-f64::from(i32::MAX), f64::from(i32::MAX))
        })
    }

    pub(super) fn take_scroll(&mut self) -> Option<[i32; 2]> {
        let total = self.total();
        let displacement = total.map(|axis| axis.round() as i32);
        if displacement == [0, 0] {
            if self.origin.is_none() {
                self.intent = [0., 0.];
            }
            return None;
        }
        if self.origin.is_some() {
            self.intent = [0, 1].map(|axis| total[axis] - f64::from(displacement[axis]));
            self.origin = Some(self.endpoint);
        } else {
            self.intent = [0., 0.];
        }
        Some(displacement)
    }

    pub(super) fn suppresses_pan(&self) -> bool {
        self.origin.is_some() || self.intent != [0., 0.] || self.camera_owned
    }

    pub(super) fn set_camera_owned(&mut self, owned: bool) {
        self.camera_owned = owned;
    }
}

#[derive(Default)]
struct Owners {
    foreign_input: bool,
    foreign_presentation: bool,
    publication: bool,
    camera_children: bool,
}

impl Owners {
    fn blocks(&self, camera_owned: bool) -> bool {
        self.foreign_input
            || self.foreign_presentation
            || (!camera_owned && (self.publication || self.camera_children))
    }
}

impl Viewer {
    pub(super) fn viewport_drag_blocked(&self) -> bool {
        self.error.is_some()
            || self.options_open
            || self.diagnostics
            || self.application_confirmation.is_some()
            || self.device.is_none()
            || self.scene.is_none()
            || self.current_move()
            || self.viewport_enter_requested
            || !self.move_inputs.is_empty()
            || self
                .client
                .as_ref()
                .and_then(|client| client.live_panel())
                .is_some_and(|panel| panel.popup().is_some())
            || self.session.as_ref().is_none_or(|session| {
                let other_input = session.purchase_cursor_pending()
                    || session.mode_publication_pending()
                    || session.tool_selection_pending()
                    || session.move_tool_pending()
                    || session.move_button_pending()
                    || session.ground_input_pending()
                    || session.object_input_pending()
                    || session.simulation.object_pickup.is_some()
                    || session.simulation.object_placement.is_some()
                    || session.simulation.cursor_factory.is_some();
                session.running_menu()
                    || session.simulation.vm.world.dialogs.modal.is_some()
                    || session.simulation.vm.constructing_dialog()
                    || session.load_completion_pending()
                    || session.postload_pending()
                    || session.host.presentation.awaiting_house_open
                    || Owners {
                        foreign_input: other_input
                            || (session.player_input_pending() && !session.publication_pending()),
                        foreign_presentation: session.ui_wait_pending()
                            || session.clock_pending()
                            || session.menu_command_pending()
                            || session.transport_pending()
                            || session.person_draw_preparation_pending(),
                        publication: session.publication_pending(),
                        camera_children: self.presentation_pending()
                            || session.view_refresh_pending()
                            || session.host.ordinary.updates_pending(),
                    }
                    .blocks(self.viewport_drag.camera_owned)
            })
    }

    pub(super) fn synchronize_viewport_drag(&mut self) {
        let blocked = self.viewport_drag_blocked();
        let suppressing = self.viewport_drag.suppresses_pan();
        self.viewport_drag
            .observe(self.application_revision, blocked);
        if suppressing {
            self.pan = [0, 0];
        }
        if !self.presentation_pending() {
            self.viewport_drag.set_camera_owned(false);
        }
    }

    pub(super) fn prepare_viewport_drag(&mut self, message: &Message) {
        self.viewport_drag.prepare(message);
        self.synchronize_viewport_drag();
    }
}

#[derive(Default)]
pub(super) struct Pointer {
    pub(super) left: Option<[f32; 2]>,
    right: bool,
    cancelled: bool,
}

fn scene_pixel(size: [u32; 2], bounds: Rectangle, cursor: mouse::Cursor) -> Option<[f32; 2]> {
    if size.contains(&0)
        || ![bounds.x, bounds.y, bounds.width, bounds.height]
            .iter()
            .all(|axis| axis.is_finite())
        || bounds.width <= 0.
        || bounds.height <= 0.
    {
        return None;
    }
    let point = cursor.position_in(bounds)?;
    let scale = (bounds.width / size[0] as f32).min(bounds.height / size[1] as f32);
    if !scale.is_finite() || scale <= 0. {
        return None;
    }
    let pixel = [
        (point.x - (bounds.width - size[0] as f32 * scale) / 2.) / scale,
        (point.y - (bounds.height - size[1] as f32 * scale) / 2.) / scale,
    ];
    pixel
        .iter()
        .zip(size)
        .all(|(&axis, limit)| axis.is_finite() && axis >= 0. && axis < limit as f32)
        .then_some(pixel)
}

impl Pointer {
    pub(super) fn reset_right(&mut self) {
        self.right = false;
        self.cancelled = false;
    }

    pub(super) fn right_button(
        &mut self,
        event: &Event,
        size: [u32; 2],
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Message> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let pixel = scene_pixel(size, bounds, cursor)?;
                self.right = true;
                self.cancelled = self.left.is_some();
                Some(Message::ViewportRightButton {
                    down: true,
                    pixel: (!self.cancelled).then_some(pixel),
                })
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) if self.right => {
                let pixel = (!self.cancelled)
                    .then(|| scene_pixel(size, bounds, cursor))
                    .flatten();
                self.reset_right();
                Some(Message::ViewportRightButton { down: false, pixel })
            }
            _ => None,
        }
    }

    pub(super) fn motion(
        &mut self,
        event: &Event,
        size: [u32; 2],
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Message> {
        let pixel = match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => scene_pixel(size, bounds, cursor),
            Event::Mouse(mouse::Event::CursorLeft) => None,
            _ => return None,
        };
        if self.right && !self.cancelled && pixel.is_none() {
            self.cancelled = true;
            return Some(Message::ViewportRightButton {
                down: false,
                pixel: None,
            });
        }
        let pan = if self.right {
            [0, 0]
        } else {
            pixel.map_or([0, 0], |pixel| {
                [0, 1].map(|axis| {
                    if pixel[axis] < 20. {
                        1
                    } else if pixel[axis] > size[axis] as f32 - 20. {
                        -1
                    } else {
                        0
                    }
                })
            })
        };
        Some(Message::ViewportMotion { pixel, pan })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rectangle {
        Rectangle {
            x: 10.,
            y: 20.,
            width: 200.,
            height: 160.,
        }
    }

    fn cursor(pixel: [f32; 2]) -> mouse::Cursor {
        mouse::Cursor::Available(iced::Point::new(10. + pixel[0] * 2., 20. + pixel[1] * 2.))
    }

    fn right(down: bool) -> Event {
        Event::Mouse(if down {
            mouse::Event::ButtonPressed(mouse::Button::Right)
        } else {
            mouse::Event::ButtonReleased(mouse::Button::Right)
        })
    }

    fn motion(pixel: [f32; 2]) -> Event {
        Event::Mouse(mouse::Event::CursorMoved {
            position: cursor(pixel).position().unwrap(),
        })
    }

    fn accept(drag: &mut Drag, message: Message, ready: bool) {
        match message {
            Message::ViewportRightButton { down: true, pixel } => drag.begin(pixel, ready),
            Message::ViewportRightButton { down: false, pixel } => drag.end(pixel),
            Message::ViewportMotion { pixel, pan } => {
                assert_eq!(pan, [0, 0]);
                drag.motion(pixel);
            }
            _ => panic!("right pointer input dispatched a non-camera action"),
        }
    }

    #[test]
    fn real_right_hold_motion_and_release_use_absolute_scene_endpoints() {
        let mut pointer = Pointer::default();
        let mut drag = Drag::default();
        accept(
            &mut drag,
            pointer
                .right_button(&right(true), [100, 80], bounds(), cursor([30., 30.]))
                .unwrap(),
            true,
        );
        assert!(drag.suppresses_pan());
        assert_eq!(drag.take_scroll(), None);
        accept(
            &mut drag,
            pointer
                .motion(&motion([45., 20.]), [100, 80], bounds(), cursor([45., 20.]))
                .unwrap(),
            true,
        );
        assert_eq!(drag.take_scroll(), Some([15, -10]));
        accept(
            &mut drag,
            pointer
                .right_button(&right(false), [100, 80], bounds(), cursor([55., 25.]))
                .unwrap(),
            true,
        );
        assert_eq!(drag.take_scroll(), Some([10, 5]));
        assert_eq!(drag.take_scroll(), None);
        assert!(!drag.suppresses_pan());
        assert!(pointer.left.is_none());
        accept(
            &mut drag,
            pointer
                .motion(&motion([70., 45.]), [100, 80], bounds(), cursor([70., 45.]))
                .unwrap(),
            true,
        );
        assert_eq!(drag.take_scroll(), None);
    }

    #[test]
    fn consecutive_motion_coalescing_preserves_total_and_release_endpoint() {
        let mut dense = Drag::default();
        let mut coalesced = Drag::default();
        for drag in [&mut dense, &mut coalesced] {
            drag.begin(Some([25.25, 30.5]), true);
        }
        for endpoint in [[80., 70.], [1., 1.], [28.75, 45.25]] {
            dense.motion(Some(endpoint));
        }
        coalesced.motion(Some([28.75, 45.25]));
        assert_eq!(dense.take_scroll(), coalesced.take_scroll());
        for drag in [&mut dense, &mut coalesced] {
            drag.end(Some([40., 48.]));
        }
        assert_eq!(dense.take_scroll(), coalesced.take_scroll());
        assert_eq!(dense.take_scroll(), None);
        assert_eq!(coalesced.take_scroll(), None);
    }

    #[test]
    fn multiple_completed_gestures_accumulate_without_a_motion_queue() {
        let mut drag = Drag::default();
        drag.begin(Some([10., 10.]), true);
        drag.end(Some([20., 15.]));
        drag.begin(Some([50., 50.]), true);
        drag.end(Some([55., 40.]));
        assert_eq!(drag.take_scroll(), Some([15, -5]));
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn blocked_down_or_new_owner_discards_intent_without_replay() {
        let mut pointer = Pointer::default();
        let mut drag = Drag::default();
        accept(
            &mut drag,
            pointer
                .right_button(&right(true), [100, 80], bounds(), cursor([30., 30.]))
                .unwrap(),
            false,
        );
        drag.observe(0, false);
        accept(
            &mut drag,
            pointer
                .motion(&motion([95., 75.]), [100, 80], bounds(), cursor([95., 75.]))
                .unwrap(),
            true,
        );
        accept(
            &mut drag,
            pointer
                .right_button(&right(false), [100, 80], bounds(), cursor([95., 75.]))
                .unwrap(),
            true,
        );
        assert_eq!(drag.take_scroll(), None);
        drag.begin(Some([30., 30.]), true);
        drag.motion(Some([80., 70.]));
        drag.observe(0, true);
        drag.observe(0, false);
        drag.motion(Some([90., 70.]));
        drag.end(Some([90., 70.]));
        assert_eq!(drag.take_scroll(), None);
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn focus_leave_install_load_restart_error_and_other_inputs_reset() {
        for boundary in [
            Message::FocusLost,
            Message::ViewportLeave,
            Message::Loaded(Err("replacement".into())),
            Message::Load,
            Message::Restart,
            Message::Options,
            Message::Buy,
            Message::Scroll([32, 0]),
            Message::ViewportButton {
                down: true,
                pixel: [40., 40.],
                picked: Some(1),
            },
        ] {
            let mut drag = Drag::default();
            drag.begin(Some([10., 10.]), true);
            drag.motion(Some([30., 30.]));
            drag.set_camera_owned(true);
            drag.prepare(&boundary);
            assert!(!drag.suppresses_pan(), "{boundary:?}");
            drag.end(Some([50., 50.]));
            assert_eq!(drag.take_scroll(), None);
        }
        let mut drag = Drag::default();
        drag.begin(Some([10., 10.]), true);
        drag.end(Some([30., 30.]));
        drag.reset();
        assert_eq!(drag.take_scroll(), None);
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn shell_revision_changes_reset_even_after_a_menu_already_closed() {
        let mut drag = Drag::default();
        drag.begin(Some([10., 10.]), true);
        drag.motion(Some([20., 20.]));
        drag.observe(2, false);
        drag.motion(Some([30., 30.]));
        drag.end(Some([40., 40.]));
        assert_eq!(drag.take_scroll(), None);
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn own_camera_continuation_keeps_endpoints_until_the_next_drain() {
        let mut drag = Drag::default();
        drag.begin(Some([10., 10.]), true);
        drag.motion(Some([20., 20.]));
        assert_eq!(drag.take_scroll(), Some([10, 10]));
        drag.set_camera_owned(true);
        drag.observe(0, false);
        drag.motion(Some([25., 35.]));
        drag.end(Some([30., 40.]));
        assert!(drag.suppresses_pan());
        drag.set_camera_owned(false);
        assert_eq!(drag.take_scroll(), Some([10, 20]));
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn own_camera_publication_preserves_motion_through_release() {
        let mut drag = Drag::default();
        drag.begin(Some([10., 10.]), true);
        drag.motion(Some([20., 20.]));
        assert_eq!(drag.take_scroll(), Some([10, 10]));
        drag.set_camera_owned(true);
        let owners = Owners {
            publication: true,
            camera_children: true,
            ..Default::default()
        };
        drag.observe(0, owners.blocks(drag.camera_owned));
        drag.motion(Some([25., 35.]));
        drag.end(Some([30., 40.]));
        drag.observe(0, Owners::default().blocks(drag.camera_owned));
        drag.set_camera_owned(false);
        assert_eq!(drag.take_scroll(), Some([10, 20]));
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn foreign_ui_wait_or_player_input_cancels_without_replay() {
        for owners in [
            Owners {
                foreign_presentation: true,
                ..Default::default()
            },
            Owners {
                foreign_input: true,
                publication: true,
                ..Default::default()
            },
        ] {
            let mut drag = Drag::default();
            drag.begin(Some([10., 10.]), true);
            drag.motion(Some([20., 20.]));
            drag.set_camera_owned(true);
            drag.observe(0, owners.blocks(drag.camera_owned));
            drag.motion(Some([25., 35.]));
            drag.end(Some([30., 40.]));
            drag.observe(0, Owners::default().blocks(drag.camera_owned));
            assert_eq!(drag.take_scroll(), None);
            assert!(!drag.suppresses_pan());
        }
    }

    #[test]
    fn letterboxing_and_invalid_nonfinite_geometry_cannot_start_dragging() {
        let wide = Rectangle {
            width: 400.,
            ..bounds()
        };
        assert_eq!(
            scene_pixel(
                [100, 80],
                wide,
                mouse::Cursor::Available(iced::Point::new(150., 80.))
            ),
            Some([20., 30.])
        );
        assert_eq!(scene_pixel([100, 80], wide, cursor([10., 30.])), None);
        for invalid in [
            Rectangle {
                width: 0.,
                ..bounds()
            },
            Rectangle {
                height: -1.,
                ..bounds()
            },
            Rectangle {
                x: f32::NAN,
                ..bounds()
            },
            Rectangle {
                y: f32::INFINITY,
                ..bounds()
            },
            Rectangle {
                width: f32::INFINITY,
                ..bounds()
            },
            Rectangle {
                height: f32::NAN,
                ..bounds()
            },
        ] {
            let mut pointer = Pointer::default();
            assert!(
                pointer
                    .right_button(&right(true), [100, 80], invalid, cursor([30., 30.]))
                    .is_none()
            );
            assert!(!pointer.right);
        }
        assert_eq!(scene_pixel([0, 80], bounds(), cursor([30., 30.])), None);
        assert_eq!(scene_pixel([100, 0], bounds(), cursor([30., 30.])), None);
        assert_eq!(
            scene_pixel([100, 80], bounds(), cursor([f32::NAN, 30.])),
            None
        );
        assert_eq!(
            scene_pixel([100, 80], bounds(), cursor([30., f32::INFINITY])),
            None
        );
        assert_eq!(
            scene_pixel([100, 80], bounds(), mouse::Cursor::Unavailable),
            None
        );
    }

    #[test]
    fn invalid_endpoint_discards_pending_and_fractional_release_does_not_stick() {
        for endpoint in [None, Some([f32::NAN, 10.]), Some([10., f32::INFINITY])] {
            let mut drag = Drag::default();
            drag.begin(Some([10., 10.]), true);
            drag.motion(Some([30., 30.]));
            drag.motion(endpoint);
            assert_eq!(drag.take_scroll(), None);
            assert!(!drag.suppresses_pan());
            drag.begin(endpoint, true);
            assert!(!drag.suppresses_pan());
        }
        let mut drag = Drag::default();
        drag.begin(Some([10., 10.]), true);
        drag.end(Some([10.25, 10.25]));
        assert_eq!(drag.take_scroll(), None);
        assert!(!drag.suppresses_pan());
        drag.begin(Some([10., 10.]), true);
        drag.end(Some([11.25, 10.]));
        assert_eq!(drag.take_scroll(), Some([1, 0]));
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn leaving_is_a_noncoalesced_cancel_and_never_resumes_edge_pan_while_held() {
        let mut pointer = Pointer::default();
        let mut drag = Drag::default();
        accept(
            &mut drag,
            pointer
                .right_button(&right(true), [100, 80], bounds(), cursor([30., 30.]))
                .unwrap(),
            true,
        );
        accept(
            &mut drag,
            pointer
                .motion(&motion([95., 75.]), [100, 80], bounds(), cursor([95., 75.]))
                .unwrap(),
            true,
        );
        let cancel = pointer
            .motion(
                &Event::Mouse(mouse::Event::CursorLeft),
                [100, 80],
                bounds(),
                mouse::Cursor::Unavailable,
            )
            .unwrap();
        assert!(matches!(
            cancel,
            Message::ViewportRightButton {
                down: false,
                pixel: None
            }
        ));
        accept(&mut drag, cancel, true);
        for pixel in [[-1., 10.], [99., 79.], [0., 0.]] {
            accept(
                &mut drag,
                pointer
                    .motion(&motion(pixel), [100, 80], bounds(), cursor(pixel))
                    .unwrap(),
                true,
            );
        }
        accept(
            &mut drag,
            pointer
                .right_button(&right(false), [100, 80], bounds(), cursor([0., 0.]))
                .unwrap(),
            true,
        );
        assert_eq!(drag.take_scroll(), None);
        assert!(!drag.suppresses_pan());
    }

    #[test]
    fn right_gesture_never_changes_left_press_or_routes_a_selection_or_queue() {
        let mut pointer = Pointer {
            left: Some([25., 25.]),
            ..Pointer::default()
        };
        let mut drag = Drag::default();
        accept(
            &mut drag,
            pointer
                .right_button(&right(true), [100, 80], bounds(), cursor([30., 30.]))
                .unwrap(),
            true,
        );
        accept(
            &mut drag,
            pointer
                .motion(&motion([99., 79.]), [100, 80], bounds(), cursor([99., 79.]))
                .unwrap(),
            true,
        );
        accept(
            &mut drag,
            pointer
                .right_button(&right(false), [100, 80], bounds(), cursor([99., 79.]))
                .unwrap(),
            true,
        );
        assert_eq!(pointer.left, Some([25., 25.]));
        assert_eq!(drag.take_scroll(), None);
        for button in [mouse::Button::Left, mouse::Button::Middle] {
            assert!(
                pointer
                    .right_button(
                        &Event::Mouse(mouse::Event::ButtonPressed(button)),
                        [100, 80],
                        bounds(),
                        cursor([30., 30.])
                    )
                    .is_none()
            );
        }
        pointer.right_button(&right(true), [100, 80], bounds(), cursor([30., 30.]));
        pointer.reset_right();
        assert!(
            pointer
                .right_button(&right(false), [100, 80], bounds(), cursor([30., 30.]))
                .is_none()
        );
        assert_eq!(pointer.left, Some([25., 25.]));
    }

    #[test]
    fn accumulated_intent_is_bounded_to_one_signed_pixel_command() {
        let mut drag = Drag::default();
        drag.begin(Some([0., 0.]), true);
        drag.end(Some([f32::MAX, -f32::MAX]));
        assert_eq!(drag.take_scroll(), Some([i32::MAX, -i32::MAX]));
        assert_eq!(drag.take_scroll(), None);
        assert!(!drag.suppresses_pan());
    }
}
