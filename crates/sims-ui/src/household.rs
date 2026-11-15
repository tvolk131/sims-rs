//! What the control panel shows, read from the simulation, and the
//! portraits of the household's people.

use crate::art::Art;
use crate::{Error, Result};
use sims_content::{Catalog, Index};
use sims_formats::bitmap::{self, Image};
use sims_formats::iff::Iff;
use sims_sim::World;
use std::collections::BTreeMap;

/// Globals the clock reads (`crates/sims-vm/src/tick.rs`).
const HOURS: usize = 0;
const MINUTES: usize = 5;
/// The global that pauses the lot (`crates/sims-vm/src/tick.rs`).
const PAUSED: usize = 17;

/// A person of the household, as the panel shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Resident {
    /// The person's object ID.
    pub object: u16,
    pub guid: u32,
    pub motives: [f32; 16],
}

/// The simulation state the control panel shows, copied out of a
/// [`World`] so drawing never holds the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub hour: i16,
    pub minute: i16,
    pub funds: i32,
    pub paused: bool,
    /// The household's people on the lot, in the family's member order.
    pub residents: Vec<Resident>,
    /// The selected Sim's object ID.
    pub selected: Option<u16>,
}

impl Snapshot {
    /// Reads the snapshot. `members` is the family's member GUIDs in the
    /// order the neighborhood keeps them (`Family::members`): the world
    /// does not keep that list, and the portraits follow it. A member with
    /// no person object on the lot is left out.
    pub fn read(world: &World, members: &[u32]) -> Self {
        let global = |n: usize| world.globals.get(n).copied().unwrap_or(0);
        let mut residents = Vec::new();
        for &guid in members {
            // The first person of the type, as the original's lookup by
            // type does (crates/sims-vm/src/selection.rs).
            if let Some(object) = world
                .objects
                .values()
                .find(|o| o.guid == guid && o.person.is_some())
            {
                residents.push(Resident {
                    object: object.id,
                    guid,
                    motives: object
                        .person
                        .as_ref()
                        .map(|p| p.motives)
                        .unwrap_or_default(),
                });
            }
        }
        Self {
            hour: global(HOURS),
            minute: global(MINUTES),
            funds: world.budget.funds,
            paused: global(PAUSED) != 0,
            residents,
            selected: world
                .objects
                .values()
                .find(|o| o.selected && o.person.is_some())
                .map(|o| o.id),
        }
    }

    pub fn resident(&self, object: u16) -> Option<&Resident> {
        self.residents.iter().find(|r| r.object == object)
    }

    /// The clock as the panel writes it: `AM 6:30`. Midnight and noon
    /// are 12. Checked against the original at 6:30 and 10:10 in the
    /// morning and 7:31 and 7:55 in the evening (`PM 7:31`); midnight
    /// and noon are **unverified**.
    pub fn clock(&self) -> String {
        let hour = self.hour.rem_euclid(24);
        let twelve = match hour % 12 {
            0 => 12,
            h => h,
        };
        let half = if hour < 12 { "AM" } else { "PM" };
        format!("{half} {twelve}:{:02}", self.minute.clamp(0, 59))
    }

    /// The funds as the panel writes them: `$2,542`. Checked against
    /// the original for that value; how it writes a debt is
    /// **unverified** (here `-$5`).
    pub fn money(&self) -> String {
        let digits = self.funds.unsigned_abs().to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        format!("{}${out}", if self.funds < 0 { "-" } else { "" })
    }
}

/// The face of a person in their character file: `BMP_` 2002 holds three
/// faces side by side, and the panel shows the first.
pub const FACES: u16 = 2002;
/// The portrait frame the face is drawn on (kPeopleTemplate): four 45 by
/// 45 cells.
pub const TEMPLATE: i32 = 4600;
/// Where the face goes in a portrait cell, and its size: the old
/// prototype's layout (legacy/crates/sims-app/src/original_hud.rs
/// `Thumbnails`, `[45, 45, 5, 2, 35, 41]`).
const FACE: [u32; 4] = [5, 2, 35, 41];

/// The faces of people, by the GUID of their type.
#[derive(Debug, Clone, Default)]
pub struct Faces {
    faces: BTreeMap<u32, Image>,
}

impl Faces {
    /// Reads the face of each of `guids` from its character file. A type
    /// with no file or no faces is left out; a face that does not decode
    /// is an error.
    pub fn load(index: &Index, catalog: &Catalog, guids: &[u32]) -> Result<Self> {
        let mut faces = BTreeMap::new();
        for &guid in guids {
            let Some(object) = catalog.object(guid) else {
                continue;
            };
            let Some(resource) = index.resources.get(object.resource) else {
                continue;
            };
            let iff =
                Iff::parse(resource.read().map_err(Error::Content)?).map_err(Error::Format)?;
            let Some(chunk) = iff.get(b"BMP_", FACES) else {
                continue;
            };
            let image = bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?;
            if let Some(face) = image.crop(
                0,
                0,
                (image.width / 3).min(FACE[2]),
                image.height.min(FACE[3]),
            ) {
                faces.insert(guid, face);
            }
        }
        Ok(Self { faces })
    }

    pub fn insert(&mut self, guid: u32, face: Image) {
        self.faces.insert(guid, face);
    }

    pub fn face(&self, guid: u32) -> Option<&Image> {
        self.faces.get(&guid)
    }

    /// The 45 by 45 portrait of `guid` on cell `cell` of the template.
    pub fn portrait(&self, art: &Art, guid: u32, cell: u32) -> Option<Image> {
        let mut portrait = art.cell(TEMPLATE, 4, cell)?;
        let face = self.face(guid)?;
        for y in 0..face.height.min(FACE[3]) {
            for x in 0..face.width.min(FACE[2]) {
                let (Some(p), Some(i)) = (
                    face.pixel(x, y),
                    portrait.width.checked_mul(y + FACE[1]).and_then(|r| {
                        r.checked_add(x + FACE[0])?
                            .checked_mul(4)
                            .map(|i| i as usize)
                    }),
                ) else {
                    continue;
                };
                if let Some(target) = portrait.rgba.get_mut(i..i + 4) {
                    target.copy_from_slice(&p);
                }
            }
        }
        Some(portrait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(hour: i16, minute: i16, funds: i32) -> Snapshot {
        Snapshot {
            hour,
            minute,
            funds,
            paused: false,
            residents: Vec::new(),
            selected: None,
        }
    }

    #[test]
    fn clock_and_money_text() {
        assert_eq!(snapshot(6, 30, 0).clock(), "AM 6:30");
        assert_eq!(snapshot(0, 5, 0).clock(), "AM 12:05");
        assert_eq!(snapshot(12, 0, 0).clock(), "PM 12:00");
        assert_eq!(snapshot(23, 59, 0).clock(), "PM 11:59");
        assert_eq!(snapshot(-1, 99, 0).clock(), "PM 11:59");
        assert_eq!(snapshot(0, 0, 2542).money(), "$2,542");
        assert_eq!(snapshot(0, 0, 0).money(), "$0");
        assert_eq!(snapshot(0, 0, 1_234_567).money(), "$1,234,567");
        assert_eq!(snapshot(0, 0, i32::MIN).money(), "-$2,147,483,648");
    }
}
