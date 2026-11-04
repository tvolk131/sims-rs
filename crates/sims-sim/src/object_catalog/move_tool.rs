//! cMoveTool's catalog groups and activation (485780, 487ab6, 4878f3).
//! Definition identities are catalog row + 1, shared with portrait lookups.
//! Activation must execute virtual+60 before resolving the requested index.
//! Placement, teardown and purchase callbacks are separate implementations.
use super::definition::{Catalog, Entry};
use serde::{Deserialize, Serialize};
pub mod cleanup;
pub mod commit;
pub mod dispatch;
pub mod feedback;
pub mod hover;
pub mod selected;
pub mod sell;

fn word(entry: &Entry, offset: usize) -> Result<i16, String> {
    let bytes = entry
        .data
        .get(offset..offset + 2)
        .ok_or("move-tool OBJD word absent")?;
    Ok(i16::from_le_bytes(bytes.try_into().unwrap()))
}
fn guid(entry: &Entry) -> Result<u32, String> {
    let bytes = entry
        .data
        .get(0x1c..0x20)
        .ok_or("move-tool OBJD GUID absent")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}
fn identity(row: usize) -> Result<u32, String> {
    u32::try_from(row)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or("move-tool definition identity overflow".into())
}
fn entry(catalog: &Catalog, id: u32) -> Result<&Entry, String> {
    id.checked_sub(1)
        .and_then(|i| catalog.entries.get(i as usize))
        .ok_or("move-tool definition absent".into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub guids: Vec<u32>,
    pub first: i32,
    pub last: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Groups(pub [Group; 7]);
impl Default for Groups {
    fn default() -> Self {
        // Static initializers48563d,485684,...,485756 ->48565d.
        Self(std::array::from_fn(|i| Group {
            guids: vec![],
            first: -20000 + i as i32 * 2000,
            last: -18001 + i as i32 * 2000,
        }))
    }
}
impl Groups {
    /// Only the first list gates initialization. Empty first lists therefore
    /// allow another constructor to append the other lists again, as native.
    /// Read borrowed normalized OBJD bytes in retained registration order.
    pub fn populate(&mut self, catalog: &Catalog) -> Result<(), String> {
        if !self.0[0].guids.is_empty() {
            return Ok(());
        }
        if catalog.entries.len() > i32::MAX as usize {
            return Err("move-tool catalog count exceeds native range".into());
        }
        for e in &catalog.entries {
            if word(e, 0x4e)? == 0 {
                let group = word(e, 0x8a)?;
                if (1..=7).contains(&group) {
                    self.0[group as usize - 1].guids.push(guid(e)?);
                }
            }
        }
        Ok(())
    }
    /// 487ab6: wrapping subtraction, signed-negative rejection, then actual
    /// vector length. The stored upper range endpoint is *not* consulted.
    pub fn lookup(
        &self,
        group: usize,
        index: i32,
        catalog: &Catalog,
    ) -> Result<Option<u32>, String> {
        let g = self.0.get(group).ok_or("move-tool group absent")?;
        let n = index.wrapping_sub(g.first);
        if n < 0 {
            return Ok(None);
        }
        let Some(guid) = g.guids.get(n as usize) else {
            return Ok(None);
        };
        catalog
            .guids
            .get(guid)
            .map(|&row| {
                let id = identity(row)?;
                entry(catalog, id)?;
                Ok(id)
            })
            .transpose()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    ///Native budget category at+24, also read by4929fb. The historical name
    ///is retained for saved projections; this is not the mouse cursor image.
    pub cursor: i32,
    pub special: u8,             // +28
    pub mode: i32,               // +34
    pub argument: i32,           // +38
    pub definition: Option<u32>, // +44
    pub position: [i32; 2],      // +50,+54
    /// +75,+76,+77,+78,+94. Names await the placement/cursor consumers.
    pub flags: [u8; 5],
    pub subtype: u8, // +ad
}
impl Fields {
    /// 487241, reached through the selected tool's virtual+3c by4cee3d.
    /// An empty hand preserves the previous offsets. Ordinary held furniture
    /// clears them; door/window offsets need their concrete door and view data.
    pub fn camera_changed(&mut self, held: bool) -> Result<(), String> {
        if !held {
            return Ok(());
        }
        if matches!(self.mode, 3 | 4) {
            return Err("camera change requires original door/window offsets487241".into());
        }
        self.position = [0; 2];
        Ok(())
    }

    /// Explicit stores from49290f and485780 only. This retained subset does
    /// not construct the remaining move tool, its window or object clipboard.
    pub fn constructor_fields() -> Self {
        Self {
            cursor: 7,
            special: 0,
            mode: 1,
            argument: -1,
            definition: None,
            position: [0; 2],
            flags: [0; 5],
            subtype: 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Deactivate,
    Classify,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    argument: i16,
    stage: Stage,
}
impl Activation {
    pub fn waiting_for_deactivation(&self) -> bool {
        self.stage == Stage::Deactivate
    }
    pub fn new(argument: u32) -> Self {
        Self {
            argument: argument as i16,
            stage: Stage::Entry,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn acknowledge_deactivation(&mut self) -> Result<(), String> {
        if self.stage != Stage::Deactivate {
            return Err("move-tool deactivation has no caller".into());
        }
        self.stage = Stage::Classify;
        Ok(())
    }
    /// true requests the actual tool's virtual+60. Retry is inert until its
    /// acknowledgement. The argument survives that callback even when+38 is
    /// overwritten; the callback's changes to the remaining fields stay live.
    pub fn advance(
        &mut self,
        f: &mut Fields,
        groups: &Groups,
        catalog: &Catalog,
        lot_word34: i16,
    ) -> Result<bool, String> {
        match self.stage {
            Stage::Entry => {
                f.argument = i32::from(self.argument);
                self.stage = Stage::Deactivate;
                return Ok(true);
            }
            Stage::Deactivate => return Ok(true),
            Stage::Complete => return Ok(false),
            Stage::Classify => (),
        }
        f.subtype = 0;
        f.special = 0;
        let definition = if self.argument == -1 {
            f.mode = 1;
            f.flags = [0, 0, 0, 1, 0];
            // This write precedes the equal-identity gate below; cursor remains
            // unchanged on the -1 branch even if it used to refer to an object.
            f.definition = None;
            None
        } else {
            let mut found = None;
            for group in 0..7 {
                if let Some(id) = groups.lookup(group, i32::from(self.argument), catalog)? {
                    found = Some(id);
                    f.mode = 3 + group as i32;
                    f.flags = [1, 1, 1, u8::from(group != 3), 0];
                    f.special = u8::from(group < 2);
                    f.subtype = u8::from(group == 3);
                    break;
                }
            }
            if found.is_none() {
                //43ac3a: signed index into the same retained definition order.
                let row = i32::from(self.argument);
                found = if row >= 0 && (row as usize) < catalog.entries.len() {
                    Some(identity(row as usize)?)
                } else {
                    None
                };
                f.mode = if found.is_some() { 2 } else { 1 };
                f.flags = [u8::from(found.is_some()), 0, 0, 1, 0];
            }
            found
        };
        if definition != f.definition {
            f.definition = definition;
            f.cursor = if let Some(id) = definition {
                let e = entry(catalog, id)?;
                if word(e, 0xc0)? == 1 && lot_word34 == 0 {
                    f.flags[1] = 1;
                }
                if word(e, 0x8a)? != 0 { 7 } else { 6 }
            } else {
                6
            };
        }
        f.position = [0; 2];
        self.stage = Stage::Complete;
        Ok(false)
    }
}
