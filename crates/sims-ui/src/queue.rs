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
//!   other object, its catalog picture (0x4ccaa0).
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
//! as a tooltip (0x5ddabc) is left out: the queue keeps no names. The
//! catalog picture of an object is not made yet, so an object's icon
//! shows its frame over the dark grey of a missing picture.
//!
//! [`Interaction::icon`]: sims_formats::saved_objects::Interaction::icon
//! [`Command::CancelAction`]: crate::command::Command::CancelAction

use crate::art::Art;
use crate::frame::{Frame, Rect};
use crate::household::Faces;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;

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

    /// Draws the icons.
    pub fn draw(&self, frame: &mut Frame, art: &Art, faces: &Faces) {
        for icon in &self.icons {
            draw_icon(frame, art, faces, icon);
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

fn draw_icon(frame: &mut Frame, art: &Art, faces: &Faces, icon: &Icon) {
    let picture = match icon.picture {
        Picture::Person(guid) => faces.portrait(art, guid, 0),
        // Catalog pictures are not made yet: the frame goes over the
        // missing picture's grey.
        Picture::Object(_) => None,
    };
    let [x, y] = [icon.rect.x, icon.rect.y];
    let Some(cells) = art.image(FRAME) else {
        frame.fill(icon.rect, NO_PICTURE);
        return;
    };
    if picture.is_none() && matches!(icon.picture, Picture::Person(_)) {
        frame.fill(icon.rect, NO_PICTURE);
        return;
    }
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
            } else if let Some(picture) = &picture {
                if let Some([r, g, b, a]) = picture.pixel(dx, dy) {
                    frame.blend(px, py, [r, g, b], a);
                }
            } else {
                frame.blend(px, py, NO_PICTURE, 255);
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
    use sims_formats::bitmap::Image;

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
        queue.draw(&mut frame, &art(), &Faces::default());
        assert_eq!(frame.pixel(32 + 1, 32), Some([255, 255, 0]));
        assert_eq!(frame.pixel(82 + 1, 32), Some([128, 128, 128]));
        // Inside: the missing picture's grey. The cut corner shows what
        // was under it.
        assert_eq!(frame.pixel(32 + 20, 32 + 20), Some(NO_PICTURE));
        assert_eq!(frame.pixel(32, 32), Some([9, 9, 9]));
    }

    #[test]
    fn a_cancelled_action_shows_the_red_cross() {
        let queue = Queue {
            person: Some(1),
            pet: false,
            icons: vec![icon(32, true, true)],
        };
        let mut frame = Frame::new(200, 100, [9, 9, 9]);
        queue.draw(&mut frame, &art(), &Faces::default());
        assert_eq!(frame.pixel(32 + 20, 32 + 20), Some([255, 0, 0]));
        assert_eq!(frame.pixel(32 + 21, 32 + 20), Some(NO_PICTURE));
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
}
