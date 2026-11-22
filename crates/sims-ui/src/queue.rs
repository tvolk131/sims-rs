//! The selected Sim's actions, as icons across the top left of the
//! screen: the running action first, then the queued ones in order. A
//! press on an icon cancels its action.
//!
//! **What the original does** (read from its code):
//!
//! - **Which actions** (0x4ea31c): the selected Sim's running action
//!   (+0xa48), then each queued action in order. An action shows only when
//!   its icon (+0xc, [`Interaction::icon`]) names an object (0x4ea203).
//!   So the action a cancel queues, which has no icon, does not show.
//! - **The picture** (0x4ea203): for a person (definition type 2), the
//!   portrait the household's table keeps for its type, cell 0 of the
//!   portrait frame; a person outside the household has none. For any
//!   other object, its catalog picture (0x4ccaa0, [`crate::thumbnail`]);
//!   an object whose type has no catalog item has none.
//! - **The frame** (0x4cabe0, `kIconFrame`, `CPanel/Buttons/IconFrame.bmp`,
//!   two 45 by 45 cells): the picture is drawn, then the frame over it,
//!   blue showing the picture and magenta cutting the corners. A queued
//!   action takes the first, grey cell. The running action takes the
//!   second, yellow cell once person data word 39 is not 0 (it is
//!   cleared when the action starts, 0x45e71c), and the grey one before.
//!   An action with no picture shows as a dark grey square (40, 40, 40)
//!   with no frame (0x4cb065, 0x4cb3d2).
//! - **Cancelled** (0x4cb3a4, `kQueueCancelOverlay`,
//!   `Other/QueueCancel.bmp`): an action with flag 0x100, which a cancel
//!   sets on the running action, has the red cross drawn over its icon.
//! - **Where** (0x4d28df): the icons are 45 by 45, the first at (32, 32)
//!   and each next one 50 to the right.
//! - **A press** (0x4cb406, 0x4cb126): unless the icon is still sliding
//!   into place, a press on it cancels its action for the selected Sim
//!   (0x4ea3e3), once: a second press on the same icon does nothing. A
//!   press on a pet's icon (person data word 65 above 1) only plays
//!   `UI_error`. The game carries out [`Command::CancelAction`].
//!
//! **Not done here.** The icons slide into their places over 500 ms
//! (0x4cb1df, 0x4cb260), and a new action's icon starts where its pie
//! menu was; here they stand in their places at once. The action's name
//! as a tooltip (0x5ddabc) is left out: the queue keeps no names. An
//! object without a thumbnail of its own is not rendered into one
//! ([`crate::thumbnail`]), so its icon shows the catalog's empty
//! template.
//!
//! [`Interaction::icon`]: sims_formats::saved_objects::Interaction::icon
//! [`Command::CancelAction`]: crate::command::Command::CancelAction

use crate::art::Art;
use crate::frame::{Frame, Rect};
use crate::household::Faces;
use crate::thumbnail::Thumbnails;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;
use std::collections::BTreeMap;

/// `kIconFrame` and `kQueueCancelOverlay` (`UIGraphics/Res_CPanel.h`,
/// `Res_Other.h`).
pub const FRAME: i32 = 4200;
pub const CANCEL_OVERLAY: i32 = 9004;

/// Every resource the queue draws, for [`Art::load`].
pub const RESOURCES: [i32; 2] = [FRAME, CANCEL_OVERLAY];

/// The icons' size, where the first one is, and how far apart they are.
pub const SIZE: i32 = 45;
pub const FIRST: [i32; 2] = [32, 32];
pub const STEP: i32 = 50;

/// Default duration in milliseconds of an action-queue icon slide (`0x4cb1ea`
/// `[0x6c7834]` = 500 ms, `0x4cb1df..0x4cb30b`).
pub const SLIDE_DURATION_MS: u32 = 500;

/// The action flag a cancel sets on the running action.
pub const CANCELLED: u32 = 0x100;

/// Person data word 39, cleared when an action starts.
const UNDER_WAY: usize = 39;
/// Person data word 65: the kind of person; above 1 is a pet.
const KIND: usize = 65;
/// The color of an icon without a picture (0x4cb096).
const NO_PICTURE: [u8; 3] = [40, 40, 40];
/// The frame's color that shows the picture under it.
const SHOW: [u8; 3] = [0, 0, 255];

/// What an icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picture {
    /// A person's portrait, by the GUID of its type.
    Person(u32),
    /// An object's catalog picture, by the GUID of its type.
    Object(u32),
}

/// One action's icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    /// The action's number, which a cancel names.
    pub uid: i32,
    pub picture: Picture,
    /// The frame's yellow cell: the running action once under way.
    pub yellow: bool,
    /// The action has flag 0x100: the red cross shows.
    pub cancelled: bool,
    pub rect: Rect,
}

/// The selected Sim's queue as the interface shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queue {
    /// The person whose actions they are.
    pub person: Option<u16>,
    /// The person is a pet, whose actions a press does not cancel.
    pub pet: bool,
    pub icons: Vec<Icon>,
}

impl Queue {
    /// Reads `person`'s actions from `world` and places their icons.
    pub fn read(world: &World, person: Option<u16>) -> Self {
        let Some(id) = person else {
            return Self::default();
        };
        let Some(me) = world.objects.get(&id).and_then(|o| o.person.as_ref()) else {
            return Self::default();
        };
        let word = |i: usize| me.data.get(i).copied().unwrap_or(0);
        let under_way = word(UNDER_WAY) != 0;
        let actions = std::iter::once((&me.active, under_way))
            .chain(me.queue.iter().map(|action| (action, false)));
        let mut icons = Vec::new();
        for (action, yellow) in actions {
            let Some(picture) = picture(world, action) else {
                continue;
            };
            let at = i32::try_from(icons.len()).unwrap_or(i32::MAX);
            let x = FIRST[0].saturating_add(STEP.saturating_mul(at));
            icons.push(Icon {
                uid: action.uid,
                picture,
                yellow,
                cancelled: action.flags & CANCELLED != 0,
                rect: Rect::new(x, FIRST[1], SIZE, SIZE),
            });
        }
        Self {
            person,
            pet: word(KIND) > 1,
            icons,
        }
    }

    /// The icon at `point`, if any.
    pub fn at(&self, point: [i32; 2]) -> Option<&Icon> {
        self.icons
            .iter()
            .find(|icon| icon.rect.contains(point[0], point[1]))
    }

    /// Like [`Self::at`], ignoring any icon currently mid-slide (`0x4cb406`:
    /// `cmp byte ptr [ecx + 0xf0], 0; jne skip`).
    pub fn at_clickable(
        &self,
        point: [i32; 2],
        slide: &QueueSlideState,
        now_ms: u32,
    ) -> Option<&Icon> {
        self.icons.iter().find(|icon| {
            !slide.is_sliding(icon.uid, now_ms) && icon.rect.contains(point[0], point[1])
        })
    }

    /// The GUIDs of the object types whose catalog pictures the icons
    /// show, for [`Thumbnails::read`].
    pub fn objects(&self) -> impl Iterator<Item = u32> + '_ {
        self.icons.iter().filter_map(|icon| match icon.picture {
            Picture::Object(guid) => Some(guid),
            Picture::Person(_) => None,
        })
    }

    /// Draws the icons.
    pub fn draw(&self, frame: &mut Frame, art: &Art, faces: &Faces, thumbnails: &Thumbnails) {
        for icon in &self.icons {
            draw_icon(frame, art, faces, thumbnails, icon);
        }
    }
}

/// Per-icon linear slide ramp (`0x4cb190..0x4cb30b`, `cRZLinearRamp` at
/// `[esi + 0xe0]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IconSlide {
    start: [i32; 2],
    target: [i32; 2],
    start_ms: u32,
    end_ms: u32,
}

impl IconSlide {
    fn pos(&self, now_ms: u32) -> [i32; 2] {
        if now_ms >= self.end_ms || self.end_ms <= self.start_ms {
            return self.target;
        }
        if now_ms <= self.start_ms {
            return self.start;
        }
        let dur = (self.end_ms - self.start_ms) as f64;
        let rem = (self.end_ms - now_ms) as f64;
        let t = (1.0 - rem / dur).clamp(0.0, 1.0);
        // 0x4cb2b4..0x4cb2f5: `round((1 - t) * start + t * target)`
        let interp = |s: i32, e: i32| -> i32 {
            ((1.0 - t) * f64::from(s) + t * f64::from(e) + 0.5).floor() as i32
        };
        [
            interp(self.start[0], self.target[0]),
            interp(self.start[1], self.target[1]),
        ]
    }

    fn is_sliding(&self, now_ms: u32) -> bool {
        now_ms < self.end_ms && self.end_ms > self.start_ms
    }
}

/// Pure-state action-queue icon slide-in & slot-shift animator matching
/// `cWinActionQueueIcon` (`0x4cb190..0x4cb30b`, `0x4cb406`).
///
/// Tracks each queued action by its [`Icon::uid`]:
/// - New actions can slide in from the pie-menu screen position over
///   [`SLIDE_DURATION_MS`] (500 ms) via [`Self::sync`].
/// - Existing actions whose target slot shifts left (when the front action
///   finishes or an earlier action is cancelled) smoothly slide from their
///   current position to the new slot over [`SLIDE_DURATION_MS`].
/// - Switching to a different selected Sim snaps all icons immediately
///   (`0x4cb190`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueSlideState {
    person: Option<u16>,
    icons: BTreeMap<i32, IconSlide>,
}

impl QueueSlideState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Places `uid` immediately at `pos` without sliding (`0x4cb190`).
    pub fn snap(&mut self, uid: i32, pos: [i32; 2]) {
        self.icons.insert(
            uid,
            IconSlide {
                start: pos,
                target: pos,
                start_ms: 0,
                end_ms: 0,
            },
        );
    }

    /// Starts a linear slide for `uid` from `from` to `to` over `duration_ms`
    /// (`0` selects [`SLIDE_DURATION_MS`], matching `0x4cb1e4..0x4cb1ea`).
    pub fn slide_to(
        &mut self,
        uid: i32,
        from: [i32; 2],
        to: [i32; 2],
        now_ms: u32,
        duration_ms: u32,
    ) {
        let dur = if duration_ms == 0 {
            SLIDE_DURATION_MS
        } else {
            duration_ms
        };
        if from == to {
            self.snap(uid, to);
            return;
        }
        self.icons.insert(
            uid,
            IconSlide {
                start: from,
                target: to,
                start_ms: now_ms,
                end_ms: now_ms.saturating_add(dur),
            },
        );
    }

    /// Synchronizes slide ramps with the latest `queue` snapshot at `now_ms`.
    ///
    /// When `origin_for_new` is `Some(click_pos)` (the pie-menu click position)
    /// and the selected Sim has not changed, newly added icons slide from
    /// `[click_pos[0] - 16, click_pos[1] - 16]` (`0x4d2adb..0x4d2af5`) to their
    /// target slot over [`SLIDE_DURATION_MS`]; otherwise they snap.
    pub fn sync(&mut self, queue: &Queue, origin_for_new: Option<[i32; 2]>, now_ms: u32) {
        let same_person = self.person == queue.person && queue.person.is_some();
        self.person = queue.person;
        if !same_person {
            self.icons.clear();
            for icon in &queue.icons {
                self.snap(icon.uid, [icon.rect.x, icon.rect.y]);
            }
            return;
        }

        let live_uids: Vec<i32> = queue.icons.iter().map(|i| i.uid).collect();
        self.icons.retain(|uid, _| live_uids.contains(uid));

        for icon in &queue.icons {
            let target = [icon.rect.x, icon.rect.y];
            if let Some(existing) = self.icons.get(&icon.uid).copied() {
                if existing.target != target {
                    let cur = existing.pos(now_ms);
                    self.slide_to(icon.uid, cur, target, now_ms, SLIDE_DURATION_MS);
                }
            } else if let Some([cx, cy]) = origin_for_new {
                let from = [cx.saturating_sub(16), cy.saturating_sub(16)];
                self.slide_to(icon.uid, from, target, now_ms, SLIDE_DURATION_MS);
            } else {
                self.snap(icon.uid, target);
            }
        }
    }

    /// Returns the interpolated `[x, y]` top-left position of `uid` at `now_ms`.
    pub fn position(&self, uid: i32, now_ms: u32) -> Option<[i32; 2]> {
        self.icons.get(&uid).map(|s| s.pos(now_ms))
    }

    /// Whether `uid` is still sliding at `now_ms` (`0x4cb406`).
    pub fn is_sliding(&self, uid: i32, now_ms: u32) -> bool {
        self.icons.get(&uid).is_some_and(|s| s.is_sliding(now_ms))
    }

    /// Whether any tracked icon is currently mid-slide at `now_ms`.
    pub fn is_animating(&self, now_ms: u32) -> bool {
        self.icons.values().any(|s| s.is_sliding(now_ms))
    }

    /// Writes each tracked icon's interpolated `[x, y]` into `queue.icons`.
    pub fn apply_to(&self, queue: &mut Queue, now_ms: u32) {
        for icon in &mut queue.icons {
            if let Some([x, y]) = self.position(icon.uid, now_ms) {
                icon.rect.x = x;
                icon.rect.y = y;
            }
        }
    }
}

/// What an action's icon shows, or None when it shows no icon: its icon
/// names no object (0x4ea218).
fn picture(world: &World, action: &Interaction) -> Option<Picture> {
    let id = u16::try_from(action.icon).ok().filter(|&id| id != 0)?;
    let object = world.objects.get(&id)?;
    Some(if object.person.is_some() {
        Picture::Person(object.guid)
    } else {
        Picture::Object(object.guid)
    })
}

fn draw_icon(frame: &mut Frame, art: &Art, faces: &Faces, thumbnails: &Thumbnails, icon: &Icon) {
    let picture = match icon.picture {
        Picture::Person(guid) => faces.portrait(art, guid, 0),
        Picture::Object(guid) => thumbnails.icon(art, guid),
    };
    let [x, y] = [icon.rect.x, icon.rect.y];
    let (Some(cells), Some(picture)) = (art.image(FRAME), picture) else {
        frame.fill(icon.rect, NO_PICTURE);
        return;
    };
    let cell = if icon.yellow { SIZE as u32 } else { 0 };
    for dy in 0..SIZE as u32 {
        for dx in 0..SIZE as u32 {
            let Some([r, g, b, a]) = cells.pixel(cell + dx, dy) else {
                continue;
            };
            let (px, py) = (x + dx as i32, y + dy as i32);
            if [r, g, b] != SHOW {
                // The frame, or a cut corner (keyed when loaded).
                frame.blend(px, py, [r, g, b], a);
            } else if let Some([r, g, b, a]) = picture.pixel(dx, dy) {
                frame.blend(px, py, [r, g, b], a);
            }
        }
    }
    if icon.cancelled
        && let Some(cross) = art.image(CANCEL_OVERLAY)
    {
        frame.draw(cross, Rect::new(0, 0, SIZE, SIZE), [x, y]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbnail::{self, Item};
    use sims_formats::bitmap::Image;

    /// The catalog template's first cell's color in [`art`].
    const TEMPLATE: [u8; 3] = [0, 0, 90];

    /// Object type 7 has a catalog item without a thumbnail; type 8 has
    /// no item.
    fn thumbnails() -> Thumbnails {
        let mut thumbnails = Thumbnails::default();
        thumbnails.insert(7, Some(Item { thumbnail: None }));
        thumbnails.insert(8, None);
        thumbnails
    }

    fn image(width: u32, height: u32, at: impl Fn(u32, u32) -> [u8; 4]) -> Image {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend(at(x, y));
            }
        }
        Image {
            width,
            height,
            rgba,
        }
    }

    /// A frame like the original's: a one-pixel ring, grey in the first
    /// cell and yellow in the second, blue inside, a cut corner at (0, 0).
    fn art() -> Art {
        let mut art = Art::default();
        art.insert(
            FRAME,
            image(90, 45, |x, y| {
                let (cx, cell) = (x % 45, x / 45);
                if (cx, y) == (0, 0) {
                    [255, 0, 255, 0]
                } else if cx == 0 || y == 0 || cx == 44 || y == 44 {
                    if cell == 0 {
                        [128, 128, 128, 255]
                    } else {
                        [255, 255, 0, 255]
                    }
                } else {
                    [0, 0, 255, 255]
                }
            }),
        );
        art.insert(
            thumbnail::TEMPLATE,
            image(180, 45, |_, _| {
                let [r, g, b] = TEMPLATE;
                [r, g, b, 255]
            }),
        );
        art.insert(
            CANCEL_OVERLAY,
            image(45, 45, |x, y| {
                if x == y {
                    [255, 0, 0, 255]
                } else {
                    [255, 0, 255, 0]
                }
            }),
        );
        art
    }

    fn icon(x: i32, yellow: bool, cancelled: bool) -> Icon {
        Icon {
            uid: 1,
            picture: Picture::Object(7),
            yellow,
            cancelled,
            rect: Rect::new(x, 32, SIZE, SIZE),
        }
    }

    #[test]
    fn the_running_action_under_way_takes_the_yellow_frame_and_queued_ones_the_grey() {
        let queue = Queue {
            person: Some(1),
            pet: false,
            icons: vec![icon(32, true, false), icon(82, false, false)],
        };
        let mut frame = Frame::new(200, 100, [9, 9, 9]);
        queue.draw(&mut frame, &art(), &Faces::default(), &thumbnails());
        assert_eq!(frame.pixel(32 + 1, 32), Some([255, 255, 0]));
        assert_eq!(frame.pixel(82 + 1, 32), Some([128, 128, 128]));
        // Inside: the catalog picture, here the bare template. The cut
        // corner shows what was under it.
        assert_eq!(frame.pixel(32 + 20, 32 + 20), Some(TEMPLATE));
        assert_eq!(frame.pixel(32, 32), Some([9, 9, 9]));
    }

    #[test]
    fn an_object_without_a_catalog_item_shows_a_dark_grey_square() {
        let mut icon = icon(32, true, false);
        icon.picture = Picture::Object(8);
        let queue = Queue {
            person: Some(1),
            pet: false,
            icons: vec![icon],
        };
        let mut frame = Frame::new(200, 100, [9, 9, 9]);
        queue.draw(&mut frame, &art(), &Faces::default(), &thumbnails());
        // No frame, no cut corner.
        assert_eq!(frame.pixel(32, 32), Some(NO_PICTURE));
        assert_eq!(frame.pixel(32 + 1, 32), Some(NO_PICTURE));
        assert_eq!(frame.pixel(32 + 20, 32 + 20), Some(NO_PICTURE));
    }

    #[test]
    fn a_cancelled_action_shows_the_red_cross() {
        let queue = Queue {
            person: Some(1),
            pet: false,
            icons: vec![icon(32, true, true)],
        };
        let mut frame = Frame::new(200, 100, [9, 9, 9]);
        queue.draw(&mut frame, &art(), &Faces::default(), &thumbnails());
        assert_eq!(frame.pixel(32 + 20, 32 + 20), Some([255, 0, 0]));
        assert_eq!(frame.pixel(32 + 21, 32 + 20), Some(TEMPLATE));
    }

    #[test]
    fn a_press_finds_the_icon_under_it() {
        let queue = Queue {
            person: Some(1),
            pet: false,
            icons: vec![icon(32, true, false), icon(82, false, false)],
        };
        assert_eq!(queue.at([32, 32]).map(|i| i.rect.x), Some(32));
        assert_eq!(queue.at([81, 50]), None);
        assert_eq!(queue.at([126, 76]).map(|i| i.rect.x), Some(82));
        assert_eq!(queue.at([127, 50]), None);
    }

    #[test]
    fn queue_slide_state_slides_over_500ms_and_blocks_clicks_while_moving() {
        let mut slide = QueueSlideState::default();
        let mut q1 = Queue {
            person: Some(1),
            pet: false,
            icons: vec![icon(32, true, false), {
                let mut i = icon(82, false, false);
                i.uid = 2;
                i
            }],
        };
        // First sync for person 1 snaps icons to their slots without sliding.
        slide.sync(&q1, None, 1000);
        assert!(!slide.is_animating(1000));
        assert_eq!(slide.position(1, 1000), Some([32, 32]));
        assert_eq!(slide.position(2, 1000), Some([82, 32]));

        // Front icon finishes -> icon 2 shifts from x=82 to x=32 over 500 ms.
        q1.icons.remove(0);
        q1.icons[0].rect.x = 32;
        slide.sync(&q1, None, 2000);
        assert!(slide.is_animating(2000));
        assert!(slide.is_sliding(2, 2250));
        assert_eq!(slide.position(2, 2000), Some([82, 32]));
        assert_eq!(slide.position(2, 2250), Some([57, 32]));
        assert_eq!(slide.position(2, 2500), Some([32, 32]));
        assert!(!slide.is_sliding(2, 2500));

        // While sliding (`0x4cb406`), `at_clickable` ignores the moving icon.
        let mut moving = q1.clone();
        slide.apply_to(&mut moving, 2250);
        assert_eq!(moving.icons[0].rect.x, 57);
        assert_eq!(moving.at_clickable([65, 40], &slide, 2250), None);
        // Once settled, `at_clickable` hits it normally.
        let mut settled = q1.clone();
        slide.apply_to(&mut settled, 2500);
        assert_eq!(settled.icons[0].rect.x, 32);
        assert_eq!(
            settled.at_clickable([40, 40], &slide, 2500).map(|i| i.uid),
            Some(2)
        );

        // Newly queued icon slides from `[click_x - 16, click_y - 16]` (`0x4d2adb..0x4d2af5`).
        let mut added = icon(82, false, false);
        added.uid = 3;
        q1.icons.push(added);
        slide.sync(&q1, Some([200, 150]), 3000);
        assert_eq!(slide.position(3, 3000), Some([184, 134]));
        assert_eq!(slide.position(3, 3250), Some([133, 83]));
        assert_eq!(slide.position(3, 3500), Some([82, 32]));
    }
}
