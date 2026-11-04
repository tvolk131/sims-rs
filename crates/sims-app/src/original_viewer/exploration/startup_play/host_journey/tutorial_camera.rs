use super::super::super::super::ui_publication::JourneyCameraWatch;
use super::super::super::super::viewport_drag::Pointer;
use super::*;
use iced::{Point, Rectangle, mouse};
use sims_render::dialog::client::buttons::Key;
use sims_sim::pose::portrait::view::Input;
use sims_vm::dialog::Record;

#[path = "tutorial_camera/go_here.rs"]
mod go_here;

const SOURCE: &str = "GameData_ALL_Recursive/Objects/Objects.far::Tutorial.iff";
const TRANSITION_FRAMES: u32 = 256;
const TOTAL_FRAMES: u32 = 2048;
const LIVE_EXPLANATION_BODY_SHA256: &str =
    "eb96930acaa78d216a8024d680de42891aabd025171d9114cd4d7aec79d7f178";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lesson {
    Zoom,
    Medium,
    Edge,
    Right,
    Rotate,
    Live,
    Bob,
    GoHere,
}

impl Lesson {
    fn digest(self) -> &'static str {
        match self {
            Self::Zoom => "1750fa43fd9116f0ab9fa27e6672253254b22609b03f5a7997c90856f5f8bb39",
            Self::Medium => "3089620aab9124e700faa38c03b11f08f03af4596949cb6c1678375c5c2a23f4",
            Self::Edge => "8b6ab98fa35768d41d6c78096e7adb46fca68239e6acf19b5f7951fb003d8c62",
            Self::Right => "abeb6a30a3ea59df062afb914d72fd442f920293f5b32a02afdc3bd6d0fd58f5",
            Self::Rotate => "9d4cc40972150f94446a8899ced264ba6a4a799f6bb0bbb603f9354a5948d7c9",
            Self::Live => "163eaba238dbebb1c29abc71ea7805eedee1e318885e9eac81b3037728637018",
            Self::Bob => "1a918e165ac7c4734425b55f3670a5ee5a3320bc2b595609302c288ffaf57247",
            Self::GoHere => "dfb2dd6d699f3eebd221b6c057557d8a456440f3f66a2c8c1b917246c037d10d",
        }
    }

    fn origin(self) -> Option<(u16, usize)> {
        match self {
            Self::Zoom => Some((4116, 1)),
            Self::Edge => Some((4116, 9)),
            Self::Rotate => Some((4116, 11)),
            Self::Live => Some((4116, 20)),
            Self::Bob => Some((4125, 1)),
            Self::GoHere => Some((4125, 3)),
            Self::Medium | Self::Right => None,
        }
    }

    fn icon(self) -> Option<Option<u16>> {
        match self {
            Self::Zoom => Some(Some(5019)),
            Self::Edge => Some(Some(5020)),
            Self::Rotate => Some(Some(5015)),
            Self::Live => Some(Some(5007)),
            Self::Bob => Some(None),
            Self::GoHere => Some(Some(5001)),
            Self::Medium | Self::Right => None,
        }
    }

    fn operands(self) -> Option<[u8; 8]> {
        match self {
            Self::Zoom => Some([0, 19, 39, 0, 0, 4, 0, 135]),
            Self::Rotate => Some([0, 15, 8, 0, 0, 4, 0, 135]),
            Self::Live => Some([0, 7, 11, 0, 0, 4, 0, 135]),
            Self::Bob => Some([0, 5, 13, 0, 0, 4, 0, 133]),
            Self::GoHere => Some([0, 1, 14, 0, 0, 4, 0, 135]),
            Self::Medium | Self::Edge | Self::Right => None,
        }
    }

    fn identify(record: &Record) -> Result<Self> {
        let digest = engine::digest(record.body.as_bytes());
        let lesson = [
            Self::Zoom,
            Self::Medium,
            Self::Edge,
            Self::Right,
            Self::Rotate,
            Self::Live,
            Self::Bob,
            Self::GoHere,
        ]
        .into_iter()
        .find(|lesson| lesson.digest() == digest)
        .context("unsupported tutorial camera: unknown lesson body")?;
        ensure!(
            record.source == SOURCE && record.origin.source == SOURCE,
            "unsupported tutorial camera: source/active origin identity unavailable"
        );
        ensure!(
            record.status == 1 && !record.modal() && record.caller > 0,
            "unsupported tutorial camera: lesson is not an active nonmodal instruction"
        );
        ensure!(
            record.title == "Tutorial"
                && record.buttons.len() == 1
                && record.buttons[0].label == "OK"
                && record.buttons[0].result == 2,
            "unsupported tutorial camera: tutorial metadata mismatch"
        );
        ensure!(
            record.presentation
                == Some(sims_vm::dialog::presentation::Presentation {
                    anchored: true,
                    hide_title: true,
                    large: false,
                }),
            "unsupported tutorial camera: unverified visible presentation"
        );
        if let Some((bhav, instruction)) = lesson.origin() {
            ensure!(
                record.origin.bhav == bhav && record.origin.instruction == instruction,
                "unsupported tutorial camera: pinned origin mismatch"
            );
        }
        if let Some(icon) = lesson.icon() {
            ensure!(
                record.bitmap_icon == icon
                    && record.icon_object.is_none()
                    && record.job_icon.is_none()
                    && record.global_bitmap_icon.is_none(),
                "unsupported tutorial camera: pinned icon mismatch"
            );
        }
        if let Some(operands) = lesson.operands() {
            ensure!(
                record.operands == operands,
                "unsupported tutorial camera: pinned operands mismatch"
            );
        }
        Ok(lesson)
    }
}

fn bob_handoff(before: &Record, after: &Record) -> Result<()> {
    ensure!(
        Lesson::identify(before)? == Lesson::Bob
            && Lesson::identify(after)? == Lesson::GoHere
            && before.id > 0
            && after.id > 0
            && before.id != after.id
            && before.caller == after.caller
            && before.stack_object == after.stack_object,
        "unsupported tutorial camera: Bob did not hand off to a fresh matching Go Here instruction"
    );
    Ok(())
}

fn scroll_watch_armed(
    record: &Record,
    current_tick: u64,
    current_camera: Input,
    pending: bool,
    watch: Option<&JourneyCameraWatch>,
) -> bool {
    let Some(watch) = watch else {
        return false;
    };
    !pending
        && matches!(Lesson::identify(record), Ok(Lesson::Edge | Lesson::Right))
        && watch.tick == current_tick
        && watch.lesson.as_ref() == Some(record)
        && watch.camera == Some(current_camera)
        && watch.owner == Some(record.caller)
        && watch.owner_source.as_deref() == Some(record.source.as_str())
        && watch.query == Some([7, 0])
        && watch.poll_pending == Some(false)
        && !watch.callback_pending
        && match (watch.baseline, watch.attempts) {
            (Some(baseline), Some(attempts)) => baseline != -1 && baseline == attempts as i32,
            _ => false,
        }
}

fn rotation_watch_armed(
    record: &Record,
    current_tick: u64,
    current_camera: Input,
    pending: bool,
    watch: Option<&JourneyCameraWatch>,
) -> bool {
    let Some(watch) = watch else {
        return false;
    };
    !pending
        && matches!(Lesson::identify(record), Ok(Lesson::Rotate))
        && watch.tick == current_tick
        && watch.lesson.as_ref() == Some(record)
        && watch.camera == Some(current_camera)
        && watch.owner == Some(record.caller)
        && watch.owner_source.as_deref() == Some(record.source.as_str())
        && watch.query == Some([6, 0])
        && watch.poll_pending == Some(false)
        && !watch.callback_pending
        && match (watch.baseline, watch.world_rotation) {
            (Some(baseline), Some(rotation @ 0..=3)) => baseline == rotation as i32,
            _ => false,
        }
}

#[derive(Default)]
struct Budget {
    transition: u32,
    total: u32,
}

impl Budget {
    fn begin(&mut self) {
        self.transition = 0;
    }

    fn frame(&mut self) -> Result<()> {
        ensure!(
            self.transition < TRANSITION_FRAMES && self.total < TOTAL_FRAMES,
            "unsupported tutorial camera: frame budget exhausted (transition={}, total={})",
            self.transition,
            self.total
        );
        self.transition += 1;
        self.total += 1;
        Ok(())
    }
}

#[derive(Default)]
struct Gestures {
    recovery: bool,
    instructed: bool,
}

impl Gestures {
    fn authorize(&mut self, lesson: Lesson, edge_moved: bool) -> Result<&'static str> {
        match lesson {
            Lesson::Edge if edge_moved && !self.recovery && !self.instructed => {
                self.recovery = true;
                Ok(
                    "native-captured recovery: edge pan moved camera, same pinned edge lesson remains",
                )
            }
            Lesson::Right if !self.instructed => {
                self.instructed = true;
                Ok("observed pinned right-button instruction")
            }
            _ => anyhow::bail!("unsupported tutorial camera: unauthorized/repeated right gesture"),
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
enum LivePhase {
    #[default]
    BeforeInput,
    Explanation,
    Bob,
    Finished,
}

#[derive(Default)]
struct LiveExplanation {
    phase: LivePhase,
    input: Option<(Record, u64)>,
    resume_pending: bool,
}

impl LiveExplanation {
    fn arm(&mut self, instruction: Record, generation: u64, activated: Option<Key>) -> Result<()> {
        ensure!(
            self.phase == LivePhase::BeforeInput
                && self.input.is_none()
                && Lesson::identify(&instruction)? == Lesson::Live
                && activated == Some(Key::Control(2010)),
            "unsupported tutorial camera: Live explanation requires one genuine instructed activation"
        );
        self.input = Some((instruction, generation));
        self.phase = LivePhase::Explanation;
        Ok(())
    }

    fn generation(&self, current: u64) -> Result<()> {
        ensure!(
            self.input
                .as_ref()
                .is_none_or(|(_, generation)| *generation == current),
            "unsupported tutorial camera: Live generation changed"
        );
        Ok(())
    }

    fn modal(&self, record: &Record, modal: bool, generation: u64) -> Result<()> {
        self.generation(generation)?;
        let (instruction, _) = self
            .input
            .as_ref()
            .context("unsupported tutorial camera: Live input not observed")?;
        ensure!(
            self.phase == LivePhase::Explanation
                && modal
                && record.modal()
                && record.status == 1
                && record.id != instruction.id
                && record.caller == instruction.caller
                && record.caller > 0
                && record.stack_object == 0
                && record.source == SOURCE
                && record.origin.source == SOURCE
                && record.origin.bhav == 4097
                && record.origin.instruction == 3
                && record.operands == [0, 7, 12, 0, 0, 4, 0, 6]
                && record.title == "Tutorial"
                && engine::digest(record.body.as_bytes()) == LIVE_EXPLANATION_BODY_SHA256
                && record.buttons.len() == 1
                && record.buttons[0].label == "OK"
                && record.buttons[0].result == 2
                && record.bitmap_icon == Some(5007)
                && record.icon_object.is_none()
                && record.job_icon.is_none()
                && record.global_bitmap_icon.is_none()
                && record.portrait.is_none()
                && record.presentation
                    == Some(sims_vm::dialog::presentation::Presentation {
                        anchored: true,
                        hide_title: true,
                        large: false,
                    }),
            "unsupported tutorial camera: unexpected/repeated Live modal"
        );
        Ok(())
    }

    fn response(
        &mut self,
        record: &Record,
        modal: bool,
        generation: u64,
        playing: bool,
    ) -> Result<Message> {
        self.modal(record, modal, generation)?;
        self.phase = LivePhase::Bob;
        self.resume_pending = !playing;
        Ok(Message::Dialog(record.id, 2))
    }

    fn lesson(&self, record: &Record, lesson: Lesson, generation: u64) -> Result<()> {
        self.generation(generation)?;
        ensure!(
            Lesson::identify(record)? == lesson,
            "unsupported tutorial camera: unverified Live continuation lesson"
        );
        let (instruction, _) = self
            .input
            .as_ref()
            .context("unsupported tutorial camera: Live input not observed")?;
        ensure!(
            match lesson {
                Lesson::Live => {
                    matches!(self.phase, LivePhase::Explanation | LivePhase::Bob)
                        && record == instruction
                }
                Lesson::Bob => self.phase == LivePhase::Bob && record.caller == instruction.caller,
                _ => false,
            },
            "unsupported tutorial camera: foreign/out-of-phase Live continuation"
        );
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        ensure!(
            self.phase == LivePhase::Bob && self.resume_pending,
            "unsupported tutorial camera: unauthorized/repeated Live resume"
        );
        self.resume_pending = false;
        Ok(())
    }
}

fn live_modal_ready(
    record: &Record,
    current_tick: u64,
    current_camera: Input,
    watch: Option<&JourneyCameraWatch>,
) -> bool {
    watch.is_some_and(|watch| {
        watch.tick == current_tick
            && watch.lesson.as_ref() == Some(record)
            && watch.camera == Some(current_camera)
            && watch.owner == Some(record.caller)
            && watch.owner_source.as_deref() == Some(record.source.as_str())
            && watch.poll_pending == Some(false)
            && !watch.callback_pending
    })
}

struct CameraJourney<'a> {
    journey: &'a mut Journey,
    budget: Budget,
    pointer: Pointer,
    gestures: Gestures,
    observed: Option<Record>,
    live_explanation: LiveExplanation,
}

impl CameraJourney<'_> {
    fn publication(&self) -> &Published {
        self.journey.host.publication_for_test()
    }

    fn check(&self) -> Result<()> {
        self.journey.bounded()?;
        self.journey.healthy()?;
        let publication = self.publication();
        let generation = self.journey.host.current_generation_for_test();
        self.live_explanation.generation(generation)?;
        if publication.modal {
            self.live_explanation.modal(
                publication
                    .dialog
                    .as_ref()
                    .context("tutorial modal absent")?,
                publication.modal,
                generation,
            )?;
        }
        ensure!(
            publication.menu.is_none() && publication.native_popup.is_none(),
            "unsupported tutorial camera: unexpected menu owner"
        );
        if let Some(Err(error)) = &publication.native_selection {
            anyhow::bail!("unsupported tutorial camera: native selection getter error: {error}");
        }
        if let Some(Err(error)) = &publication.journey_camera {
            anyhow::bail!("unsupported tutorial camera: retained camera getter error: {error}");
        }
        Ok(())
    }

    fn camera(&self) -> Result<Input> {
        let input = self
            .publication()
            .journey_camera
            .as_ref()
            .context("unsupported tutorial camera: retained camera absent")?
            .as_ref()
            .copied()
            .map_err(|error| {
                anyhow::anyhow!("unsupported tutorial camera: retained camera: {error}")
            })?;
        ensure!(
            (1..=3).contains(&input.zoom) && input.rotation < 4,
            "unsupported tutorial camera: retained camera zoom/rotation out of range"
        );
        Ok(input)
    }

    fn active(&mut self, allowed: &[Lesson]) -> Result<Option<Lesson>> {
        self.check()?;
        let Some(record) = self.publication().dialog.clone() else {
            return Ok(None);
        };
        let lesson = Lesson::identify(&record)?;
        ensure!(
            allowed.contains(&lesson),
            "unsupported tutorial camera: unexpected known lesson {lesson:?}; allowed {allowed:?}"
        );
        if self.observed.as_ref() != Some(&record) {
            self.journey.events.push(json!({"kind":"tutorial_camera_lesson",
                "lesson":format!("{lesson:?}"),"record":record,
                "body_sha256":lesson.digest(),"actual_current_origin":record.origin,
                "independent_origin_pin":if lesson == Lesson::GoHere {None} else {lesson.origin()},
                "replacement_capture_origin_pin":(lesson == Lesson::GoHere).then(|| lesson.origin()),
                "pin_scope":if lesson == Lesson::GoHere {"retained Host record; not independent native capture"}
                    else if lesson.origin().is_some() {"body and known metadata"}
                    else {"native body-only capture; current origin observed, not independently pinned"},
                "response_status":"not_sent; nonmodal instruction authorizes visible player input only",
                "camera":self.camera()?,"generation":self.journey.host.current_generation_for_test()}));
            self.observed = Some(record);
        }
        Ok(Some(lesson))
    }

    fn frame(&mut self) -> Result<()> {
        self.check()?;
        let advance = !self.publication().pending && !self.publication().modal;
        ensure!(
            !advance || self.publication().playing,
            "unsupported tutorial camera: uncommanded pause"
        );
        self.budget.frame()?;
        self.journey.frame(advance)?;
        self.check()
    }

    fn wait(&mut self, previous: Option<Lesson>, expected: Lesson) -> Result<()> {
        let allowed: Vec<_> = previous.into_iter().chain([expected]).collect();
        loop {
            let current = self.active(&allowed)?;
            if !self.publication().pending && current == Some(expected) {
                return Ok(());
            }
            self.frame()?;
        }
    }

    fn settle(&mut self, allowed: &[Lesson]) -> Result<()> {
        loop {
            self.active(allowed)?;
            if !self.publication().pending {
                return Ok(());
            }
            self.frame()?;
        }
    }

    fn wait_live_bob(&mut self) -> Result<()> {
        loop {
            self.check()?;
            let publication = self.publication();
            let generation = self.journey.host.current_generation_for_test();
            let modal = publication.modal;
            let playing = publication.playing;
            if modal {
                let record = publication.dialog.as_ref().unwrap().clone();
                if live_modal_ready(
                    &record,
                    tick(publication)?,
                    self.camera()?,
                    publication.journey_camera_watch.as_ref(),
                ) {
                    ensure!(
                        self.journey.idle && self.journey.streams == 0 && self.journey.dialogs < 16,
                        "unsupported tutorial camera: Live response task/dialog fence"
                    );
                    let message = self
                        .live_explanation
                        .response(&record, modal, generation, playing)?;
                    self.journey.events.push(json!({"kind":"tutorial_camera_live_modal",
                        "record":record,"body_sha256":LIVE_EXPLANATION_BODY_SHA256,
                        "generation":generation,"response":2,"response_status":"attempted via Journey.ui",
                        "transition_frames_including_service_polls":self.budget.transition,
                        "frames_including_service_polls":self.budget.total}));
                    let receipt = self.journey.events.len() - 1;
                    self.journey.ui(message)?;
                    self.journey.dialogs += 1;
                    self.journey.events[receipt]["response_status"] = json!("task_streams_drained");
                    continue;
                }
            } else {
                if playing {
                    self.live_explanation.resume_pending = false;
                }
                let current = self.active(&[Lesson::Live, Lesson::Bob])?;
                if let Some(lesson) = current {
                    self.live_explanation.lesson(
                        self.publication().dialog.as_ref().unwrap(),
                        lesson,
                        generation,
                    )?;
                }
                if !self.publication().pending {
                    if !self.publication().playing {
                        ensure!(
                            self.publication()
                                .pause_button
                                .is_some_and(|button| button.enabled),
                            "unsupported tutorial camera: enabled Live continuation playback absent"
                        );
                        self.live_explanation.resume()?;
                        self.journey.ui(Message::Toggle)?;
                        continue;
                    }
                    if current == Some(Lesson::Bob) {
                        self.live_explanation.phase = LivePhase::Finished;
                        return Ok(());
                    }
                }
            }
            self.frame()?;
        }
    }

    fn require(&mut self, lesson: Lesson) -> Result<()> {
        ensure!(
            self.active(&[lesson])? == Some(lesson) && !self.publication().pending,
            "unsupported tutorial camera: current instruction unavailable/pending"
        );
        Ok(())
    }

    fn scroll_armed(&self) -> Result<bool> {
        let publication = self.publication();
        let Some(record) = publication.dialog.as_ref() else {
            return Ok(false);
        };
        Ok(scroll_watch_armed(
            record,
            tick(publication)?,
            self.camera()?,
            publication.pending,
            publication.journey_camera_watch.as_ref(),
        ))
    }

    fn wait_camera_watch(&mut self, lesson: Lesson) -> Result<()> {
        self.require(lesson)?;
        let record = self.publication().dialog.clone();
        loop {
            self.active(&[lesson])?;
            ensure!(
                self.publication().dialog == record,
                "unsupported tutorial camera: active lesson changed while arming scroll watch"
            );
            let armed = if lesson == Lesson::Rotate {
                let publication = self.publication();
                rotation_watch_armed(
                    publication.dialog.as_ref().unwrap(),
                    tick(publication)?,
                    self.camera()?,
                    publication.pending,
                    publication.journey_camera_watch.as_ref(),
                )
            } else {
                self.scroll_armed()?
            };
            if armed {
                let watch = self.publication().journey_camera_watch.clone();
                self.journey
                    .events
                    .push(json!({"kind":if lesson == Lesson::Rotate {
                        "tutorial_camera_rotation_ready"
                    } else {"tutorial_camera_scroll_ready"},
                    "lesson":format!("{lesson:?}"),"watch":watch,
                    "transition_frames_including_service_polls":self.budget.transition,
                    "frames_including_service_polls":self.budget.total}));
                return Ok(());
            }
            self.frame()?;
        }
    }

    fn startup(&mut self) -> Result<()> {
        self.budget.begin();
        let mut requested = false;
        let mut answered = false;
        loop {
            self.journey.bounded()?;
            self.journey.healthy()?;
            let publication = self.publication().clone();
            if publication.modal {
                let record = publication
                    .dialog
                    .as_ref()
                    .context("startup modal absent")?;
                ensure!(self.journey.dialogs < 16, "Host startup dialog limit");
                let response = startup_response(record)?;
                ensure!(
                    record.status == 1 && record.modal(),
                    "startup modal not answerable"
                );
                self.journey.events.push(json!({"kind":"tutorial_camera_startup_modal",
                    "record":record,"body_sha256":engine::digest(record.body.as_bytes()),
                    "response":response,"response_status":"attempted via unchanged startup_response"}));
                let receipt = self.journey.events.len() - 1;
                self.journey.ui(Message::Dialog(record.id, response))?;
                self.journey.events[receipt]["response_status"] = json!("task_streams_drained");
                self.journey.dialogs += 1;
                answered = true;
                continue;
            }
            let current = self.active(&[Lesson::Zoom])?;
            if current == Some(Lesson::Zoom) && !publication.pending && publication.playing {
                self.camera()?;
                return Ok(());
            }
            match next_input(
                None,
                None,
                publication.pending,
                publication.playing,
                requested,
                answered,
            )? {
                NextInput::Poll | NextInput::Frame => {
                    self.budget.frame()?;
                    self.journey.frame(!publication.pending)?;
                }
                NextInput::Play => {
                    ensure!(
                        publication
                            .pause_button
                            .is_some_and(|button| button.enabled),
                        "unsupported tutorial camera: enabled playback control absent"
                    );
                    self.journey.ui(Message::Toggle)?;
                    requested = true;
                    answered = false;
                }
                NextInput::Dialog(_) => unreachable!(),
            }
        }
    }

    fn control(
        &mut self,
        lesson: Lesson,
        id: i32,
        kind: Option<Widget>,
        message: Message,
    ) -> Result<()> {
        self.require(lesson)?;
        let publication = self.publication();
        let cluster = publication
            .cluster
            .as_ref()
            .context("unsupported tutorial camera: HUD absent")?;
        ensure!(
            cluster.values.ready
                && !cluster.values.pending
                && publication.scene.is_some()
                && publication.tutorial_highlight == Some(id)
                && cluster
                    .art
                    .as_ref()
                    .is_some_and(|art| art.hud(id).is_some()),
            "unsupported tutorial camera: highlighted rendered control {id} missing/not ready"
        );
        if let Some(kind) = kind {
            ensure!(
                publication
                    .button(kind)
                    .is_some_and(|button| button.enabled),
                "unsupported tutorial camera: native control {kind:?} disabled/missing"
            );
            let enabled = match kind {
                Widget::ZoomIn => cluster.values.buttons.zoom_in.map(|button| button.enabled),
                Widget::Live => cluster
                    .values
                    .buttons
                    .live
                    .map(|button| button.enabled)
                    .filter(|_| cluster.values.can_show_live),
                _ => None,
            };
            ensure!(
                enabled == Some(true),
                "unsupported tutorial camera: rendered control disabled"
            );
        } else {
            ensure!(
                matches!(id, 2011 | 2012),
                "unsupported tutorial camera control"
            );
        }
        let before = self.camera()?;
        let live_input = (lesson == Lesson::Live).then(|| {
            (
                publication.dialog.as_ref().unwrap().clone(),
                self.journey.host.current_generation_for_test(),
            )
        });
        self.journey.events.push(json!({"kind":"tutorial_camera_control","resource":id,
            "lesson":format!("{lesson:?}"),"camera_before":before,
            "evidence_scope":"enabled retained inputs to production rendered HUD; not GUI/pixel qualification"}));
        self.journey.ui(message)?;
        ensure!(
            self.publication()
                .journey_tutorial_buttons
                .as_ref()
                .is_some_and(|buttons| buttons.activated == Some(Key::Control(id))),
            "unsupported tutorial camera: actual control activation not retained"
        );
        if let Some((instruction, generation)) = live_input {
            ensure!(
                self.journey.host.current_generation_for_test() == generation,
                "unsupported tutorial camera: instructed Live generation changed"
            );
            let activated = self
                .publication()
                .journey_tutorial_buttons
                .as_ref()
                .and_then(|buttons| buttons.activated);
            self.live_explanation
                .arm(instruction, generation, activated)?;
        }
        self.check()
    }

    fn pointer_event(&mut self, event: iced::Event, pixel: [f32; 2]) -> Result<()> {
        self.check()?;
        let scene = self
            .publication()
            .scene
            .as_ref()
            .context("unsupported tutorial camera: scene absent")?;
        let size = [scene.width, scene.height];
        ensure!(
            size.iter().all(|&axis| axis >= 160),
            "unsupported tutorial camera: scene too small"
        );
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: size[0] as f32,
            height: size[1] as f32,
        };
        let cursor = mouse::Cursor::Available(Point::new(pixel[0], pixel[1]));
        let message = self
            .pointer
            .right_button(&event, size, bounds, cursor)
            .or_else(|| self.pointer.motion(&event, size, bounds, cursor))
            .context("unsupported tutorial camera: Pointer emitted no message")?;
        ensure!(
            matches!(
                message,
                Message::ViewportMotion { .. } | Message::ViewportRightButton { .. }
            ),
            "unsupported tutorial camera: non-camera Pointer output"
        );
        self.journey.events.push(json!({"kind":"tutorial_camera_pointer",
            "iced_event":format!("{event:?}"),"pixel":pixel,"scene_size":size,
            "bounds":[bounds.x,bounds.y,bounds.width,bounds.height],
            "emitted_message":format!("{message:?}"),"input_origin":"actual Pointer::right_button/motion"}));
        self.journey.ui(message)
    }

    fn motion(&mut self, pixel: [f32; 2]) -> Result<()> {
        self.pointer_event(
            iced::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(pixel[0], pixel[1]),
            }),
            pixel,
        )
    }

    fn point(&self) -> Result<[f32; 2]> {
        let scene = self
            .publication()
            .scene
            .as_ref()
            .context("tutorial scene absent")?;
        Ok([scene.width as f32 * 0.35, scene.height as f32 * 0.65])
    }

    fn gesture(&mut self, lesson: Lesson, edge_moved: bool) -> Result<()> {
        self.wait_camera_watch(lesson)?;
        let record = self.publication().dialog.clone();
        let authorization = self.gestures.authorize(lesson, edge_moved)?;
        let before = self.camera()?;
        self.journey
            .events
            .push(json!({"kind":"tutorial_camera_right_gesture",
            "authorization":authorization,"camera_before":before,
            "recovery_used":self.gestures.recovery,"instructed_used":self.gestures.instructed}));
        let start = self.point()?;
        self.motion(start)?;
        self.pointer_event(
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
            start,
        )?;
        self.frame()?;
        self.settle(&[lesson])?;
        ensure!(
            self.publication().dialog == record,
            "unsupported tutorial camera: active lesson changed during right-button press"
        );
        self.wait_camera_watch(lesson)?;
        let end = [start[0] + 32., start[1] + 16.];
        self.motion(end)?;
        self.frame()?;
        self.active(&[
            lesson,
            if lesson == Lesson::Edge {
                Lesson::Right
            } else {
                Lesson::Rotate
            },
        ])?;
        self.pointer_event(
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)),
            end,
        )?;
        let expected = if lesson == Lesson::Edge {
            Lesson::Right
        } else {
            Lesson::Rotate
        };
        self.wait(Some(lesson), expected)?;
        let after = self.camera()?;
        ensure!(
            after.center != before.center,
            "unsupported tutorial camera: right gesture did not move retained center"
        );
        self.journey.events.push(
            json!({"kind":"tutorial_camera_change","input":"right gesture",
            "authorization":authorization,"before":before,"after":after}),
        );
        Ok(())
    }

    fn edge(&mut self) -> Result<()> {
        self.budget.begin();
        self.wait_camera_watch(Lesson::Edge)?;
        let record = self.publication().dialog.clone();
        let before = self.camera()?;
        let neutral = self.point()?;
        self.motion([1., neutral[1]])?;
        loop {
            self.active(&[Lesson::Edge, Lesson::Right])?;
            if self.camera()?.center != before.center {
                break;
            }
            self.frame()?;
        }
        self.motion(neutral)?;
        self.settle(&[Lesson::Edge, Lesson::Right])?;
        let after = self.camera()?;
        ensure!(
            after.center != before.center,
            "unsupported tutorial camera: edge pan did not retain a center change"
        );
        self.journey
            .events
            .push(json!({"kind":"tutorial_camera_change","input":"edge pan",
            "before":before,"after":after,"neutral_pointer_sent":true}));
        loop {
            let current = self.active(&[Lesson::Edge, Lesson::Right])?;
            if !self.publication().pending {
                match current {
                    Some(Lesson::Right) => return Ok(()),
                    Some(Lesson::Edge) if self.scroll_armed()? => {
                        ensure!(
                            self.publication().dialog == record,
                            "unsupported tutorial camera: edge recovery instruction changed"
                        );
                        return self.gesture(Lesson::Edge, true);
                    }
                    _ => {}
                }
            }
            self.frame()?;
        }
    }

    fn bob(&mut self) -> Result<Value> {
        self.budget.begin();
        self.require(Lesson::Bob)?;
        let publication = self.publication();
        ensure!(
            expanded_people_panel(
                publication.panel_state,
                publication.root_visible,
                publication.people_visible
            ) && live_disclosure_available(
                authoritative_mode(publication),
                publication.button(Widget::Live)
            ),
            "unsupported tutorial camera: Bob portrait not currently visible in expanded Live"
        );
        let candidates: Vec<_> = publication
            .residents
            .iter()
            .filter(|resident| resident.name == "Bob")
            .collect();
        ensure!(
            candidates.len() == 1,
            "unsupported tutorial camera: Bob identity ambiguous/absent"
        );
        let bob = (*candidates[0]).clone();
        let record = publication
            .dialog
            .as_ref()
            .context("Bob instruction absent")?;
        ensure!(
            bob.enabled
                && record.stack_object == bob.neighbor
                && publication
                    .art
                    .as_ref()
                    .is_some_and(|art| art.resident(bob.neighbor).is_some())
                && publication
                    .journey_tutorial_buttons
                    .as_ref()
                    .is_some_and(|buttons| {
                        buttons.initialized
                            && buttons
                                .items
                                .iter()
                                .filter(|button| button.neighbor == bob.neighbor)
                                .count()
                                == 1
                            && buttons.image(bob.neighbor).is_some()
                    }),
            "unsupported tutorial camera: current enabled native Bob portrait identity unavailable"
        );
        let instruction = record.clone();
        let before = observation(publication);
        self.journey.ui(Message::Resident(bob.object))?;
        self.settle(&[Lesson::Bob, Lesson::GoHere])?;
        let publication = self.publication();
        require_native_resident(publication, bob.object)?;
        ensure!(
            publication
                .journey_tutorial_buttons
                .as_ref()
                .is_some_and(|buttons| buttons.activated == Some(Key::Resident(bob.neighbor)))
                && publication
                    .residents
                    .iter()
                    .any(|resident| resident.object == bob.object
                        && resident.neighbor == bob.neighbor
                        && resident.name == bob.name
                        && resident.enabled),
            "unsupported tutorial camera: Bob activation/current identity not retained"
        );
        self.wait(Some(Lesson::Bob), Lesson::GoHere)?;
        let publication = self.publication();
        let next = publication
            .dialog
            .as_ref()
            .context("unsupported tutorial camera: Go Here instruction absent")?;
        bob_handoff(&instruction, next)?;
        require_native_resident(publication, bob.object)?;
        let roster = publication.residents.clone();
        let coverage = resident_selection_coverage(&roster, &[bob.object]);
        let receipt = json!({"status":"passed","before":before,"after":observation(publication),
            "bob":{"object":bob.object,"neighbor":bob.neighbor,"name":bob.name},
            "next_instruction":next,"next_instruction_body_sha256":Lesson::GoHere.digest(),
            "next_instruction_response_sent":false,"go_here_execution_qualified":false,
            "camera":self.camera()?,"household_selection_coverage":coverage,
            "betty_qualified":false,"tutorial_completion_qualified":false,
            "scope":"Bob only; unchanged generic two-resident oracle still follows"});
        self.journey
            .events
            .push(json!({"kind":"tutorial_camera_bob","receipt":receipt}));
        Ok(receipt)
    }

    fn execute(&mut self) -> Result<Value> {
        self.startup()?;
        for (lesson, expected) in [
            (Lesson::Zoom, Lesson::Medium),
            (Lesson::Medium, Lesson::Edge),
        ] {
            self.budget.begin();
            let before = self.camera()?;
            self.control(lesson, 2013, Some(Widget::ZoomIn), Message::ZoomIn)?;
            self.wait(Some(lesson), expected)?;
            let after = self.camera()?;
            ensure!(
                after.zoom == before.zoom + 1,
                "unsupported tutorial camera: ZoomIn did not change retained zoom by one"
            );
            self.journey
                .events
                .push(json!({"kind":"tutorial_camera_change","input":"ZoomIn",
                "before":before,"after":after}));
        }
        self.edge()?;
        self.budget.begin();
        self.gesture(Lesson::Right, false)?;
        self.budget.begin();
        self.wait_camera_watch(Lesson::Rotate)?;
        let before = self.camera()?;
        let (id, message) = match self.publication().tutorial_highlight {
            Some(2011) => (2011, Message::Rotate),
            Some(2012) => (2012, Message::RotateBack),
            _ => {
                anyhow::bail!("unsupported tutorial camera: highlighted rotation 2011/2012 absent")
            }
        };
        self.control(Lesson::Rotate, id, None, message)?;
        self.wait(Some(Lesson::Rotate), Lesson::Live)?;
        let after = self.camera()?;
        ensure!(
            after.rotation != before.rotation,
            "unsupported tutorial camera: rotation unchanged"
        );
        self.journey.events.push(
            json!({"kind":"tutorial_camera_change","input":"highlighted rotation",
            "resource":id,"before":before,"after":after}),
        );
        self.budget.begin();
        self.control(Lesson::Live, 2010, Some(Widget::Live), Message::Live)?;
        self.wait_live_bob()?;
        let bob = self.bob()?;
        Ok(
            json!({"status":"passed","frames_including_service_polls":self.budget.total,
            "frames_per_expected_transition":TRANSITION_FRAMES,"total_frame_limit":TOTAL_FRAMES,
            "recovery_gesture_used":self.gestures.recovery,"instructed_right_gesture_used":self.gestures.instructed,
            "zoom_in_count":2,"bob":bob,"betty_qualified":false,"tutorial_completion_qualified":false,
            "window_qualified":false,"strict_original_parity_ticks":0,
            "scope":"real Host/UI/task streams and retained camera/native selection; no GUI or full tutorial claim"}),
        )
    }
}

pub(super) fn object_dependencies(
    camera: bool,
    go_here: bool,
    walking: bool,
    source: &str,
) -> Result<()> {
    ensure!(
        camera && go_here && walking && source == "UserData/Houses/House07.iff",
        "tutorial object interaction requires CAMERA=1, GO_HERE=1, WALKING=1 and fresh House07"
    );
    Ok(())
}

pub(super) fn run(journey: &mut Journey) -> Result<Value> {
    let go_here_enabled = env_bool("SIMS_HOST_JOURNEY_TUTORIAL_GO_HERE")?;
    let walking_enabled = env_bool("SIMS_HOST_JOURNEY_TUTORIAL_WALKING")?;
    let object_enabled = env_bool("SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_INTERACTION")?;
    if object_enabled {
        object_dependencies(
            env_bool("SIMS_HOST_JOURNEY_TUTORIAL_CAMERA")?,
            go_here_enabled,
            walking_enabled,
            &std::env::var("SIMS_HOST_JOURNEY_SOURCE")?,
        )?;
    }
    if walking_enabled {
        ensure!(
            go_here_enabled && env_bool("SIMS_HOST_JOURNEY_TUTORIAL_CAMERA")?,
            "tutorial walking requires CAMERA=1 and GO_HERE=1"
        );
    }
    let mut camera = CameraJourney {
        journey,
        budget: Budget::default(),
        pointer: Pointer::default(),
        gestures: Gestures::default(),
        observed: None,
        live_explanation: LiveExplanation::default(),
    };
    let result = camera.execute();
    if let Err(error) = &result {
        let watch = camera.publication().journey_camera_watch.clone();
        camera
            .journey
            .events
            .push(json!({"kind":"tutorial_camera_stop","status":"unsupported",
            "reason":format!("{error:#}"),"frames_including_service_polls":camera.budget.total,
            "transition_frames_including_service_polls":camera.budget.transition,
            "last_scroll_watch":watch,
            "no_owner_recovery_or_synthetic_completion":true}));
    }
    let mut receipt = result?;
    if go_here_enabled {
        let instruction = camera
            .observed
            .clone()
            .context("unsupported tutorial Go Here: Bob handoff record absent")?;
        if walking_enabled {
            receipt["walking"] = go_here::run_walking(camera.journey, &instruction)?;
            if let Some(object) = go_here::object_continuation(object_enabled, || {
                go_here::run_object(camera.journey, &instruction)
            })? {
                receipt["object_interaction"] = object;
                camera
                    .journey
                    .events
                    .push(json!({"kind":"tutorial_camera_object_receipt",
                    "receipt":receipt,"generic_play_sit_continuation_permitted":false}));
                anyhow::bail!(
                    "tutorial object boundary reached; no next response or generic Play/Sit"
                );
            }
            camera
                .journey
                .events
                .push(json!({"kind":"tutorial_camera_walking_receipt",
                "receipt":receipt,"generic_play_sit_continuation_permitted":false}));
            anyhow::bail!(
                "tutorial walking boundary reached; stop before any next response or generic Play/Sit"
            );
        }
        receipt["go_here"] = go_here::run(camera.journey, &instruction)?;
        camera.journey.events.push(
            json!({"kind":"tutorial_camera_go_here_receipt","receipt":receipt,
            "generic_play_sit_continuation_permitted":false}),
        );
        anyhow::bail!(
            "tutorial Go Here enqueue/first-instruction boundary reached; stop before generic Play/Sit without responding"
        );
    }
    Ok(receipt)
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "tutorial_camera/private_tests.rs"]
#[rustfmt::skip]
mod tests;
