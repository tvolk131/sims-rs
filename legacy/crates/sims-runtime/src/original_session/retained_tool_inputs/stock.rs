//! Source-defined initialized storage of the existing stock tools. These
//! fields are retained without constructors or guessed meanings. Byte ranges
//! deliberately exclude padding, conditional scratch storage and native heap
//! allocation capacity. Pointer relationships have separate logical aliases.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::object_catalog::move_tool::Groups;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub offset: u16,
    pub bytes: Vec<u8>,
}
/// Initialized scalar ranges from the qualified constructors and factory
/// virtual+34 setup. Picker target/mode/activation/cursor/actor already have
/// independent live owners and are not duplicated here. Native history and
/// preview pointers are represented by the aliases below.
pub const FIELDS: [&[(u16, usize)]; 12] = [
    &[(0x2c, 1), (0x36, 1), (0x44, 16)],
    &[(0x2c, 1), (0x36, 1), (0x44, 16)],
    &[(0x2c, 10), (0x38, 8), (0x60, 8), (0x6c, 4), (0x70, 2)],
    &[(0x2c, 4), (0x38, 16), (0x5c, 2), (0x60, 4)],
    &[(0x2c, 1), (0x36, 1), (0x44, 8), (0x4e, 1), (0x50, 8)],
    &[(0x38, 16), (0x64, 1)],
    &[
        (0x2c, 1),
        (0x30, 12),
        (0x4c, 12),
        (0xd8, 3),
        (0xdc, 8),
        (0xe8, 4),
    ],
    &[
        (0x30, 30),
        (0x50, 28),
        (0x74, 14),
        (0x84, 6),
        (0x8c, 15),
        (0x9c, 18),
    ],
    &[(0x2c, 7), (0x35, 1), (0x38, 8)],
    &[(0x2c, 4), (0x34, 18), (0x48, 4), (0x54, 4), (0x58, 2)],
    &[(0x2c, 11), (0x38, 12), (0x48, 4), (0x4c, 1), (0x54, 4)],
    &[(0x2c, 7), (0x35, 1), (0x38, 8), (0x44, 4)],
];
pub const PREVIEW_FIELDS: [&[(u16, usize)]; 3] = [
    &[
        (0xe024, 1),
        (0xe028, 4),
        (0xe02e, 1),
        (0xe032, 1),
        (0xe04a, 1),
        (0xe04d, 1),
        (0xe050, 16),
        (0xe060, 1),
    ],
    &[(0x32, 1), (0x38, 4), (0x40, 1)],
    &[],
];
pub fn project(raw: &[u8], ranges: &[(u16, usize)]) -> Vec<Field> {
    ranges
        .iter()
        .map(|&(offset, len)| Field {
            offset,
            bytes: raw[usize::from(offset)..usize::from(offset) + len].to_vec(),
        })
        .collect()
}
fn shape(fields: &[Field], expected: &[(u16, usize)]) -> Result<()> {
    ensure!(
        fields.len() == expected.len()
            && fields
                .iter()
                .zip(expected)
                .all(|(f, &(o, n))| f.offset == o && f.bytes.len() == n),
        "retained initialized stock field layout differs"
    );
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    pub slot: u8,
    pub offset: u16,
    pub handle: u32,
}
/// An actually allocated preview, with qualified empty before/after vectors
/// and empty change records. Nonempty native work cannot enter this variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyPreview {
    pub slot: u8,
    pub handle: u32,
    pub class: u32,
    pub references: u32,
    pub fields: Vec<Field>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stock {
    pub initialized: [Vec<Field>; 12],
    /// Both actual UndoManager vectors are NULL and empty, not historical
    /// unknown vectors or an empty replacement-manager constructor result.
    pub empty_history: u32,
    pub history_aliases: Vec<Alias>,
    pub previews: [EmptyPreview; 3],
    pub groups: Groups,
    pub picker_reasons: Vec<Vec<u8>>,
    pub dirt_reasons: Vec<Vec<u8>>,
    pub dirt_references: u32,
    pub picker_shared_null: bool,
}
impl Stock {
    pub fn validate(&self) -> Result<()> {
        for (fields, ranges) in self.initialized.iter().zip(FIELDS) {
            shape(fields, ranges)?;
        }
        ensure!(
            self.empty_history != 0
                && self.picker_shared_null
                && self.dirt_references > 0
                && self.dirt_references < 1024,
            "retained stock shared owners absent"
        );
        let expected = [
            (2, 0x68),
            (3, 0x34),
            (5, 0x2c),
            (7, 0x2c),
            (9, 0x30),
            (10, 0x50),
        ];
        ensure!(
            self.history_aliases.len() == expected.len()
                && self
                    .history_aliases
                    .iter()
                    .zip(expected)
                    .all(|(a, (slot, offset))| a.slot == slot
                        && a.offset == offset
                        && a.handle == self.empty_history),
            "retained history aliases differ"
        );
        for ((preview, slot), ranges) in self.previews.iter().zip([2, 3, 5]).zip(PREVIEW_FIELDS) {
            ensure!(
                preview.slot == slot
                    && preview.handle != 0
                    && preview.class != 0
                    && preview.references == 1,
                "retained empty preview ownership differs"
            );
            shape(&preview.fields, ranges)?;
        }
        ensure!(
            self.previews
                .iter()
                .enumerate()
                .all(|(i, p)| self.previews[..i].iter().all(|q| p.handle != q.handle)),
            "retained preview lifetimes alias"
        );
        ensure!(
            self.picker_reasons.len() == 9
                && self.dirt_reasons.len() == 7
                && self
                    .picker_reasons
                    .iter()
                    .chain(&self.dirt_reasons)
                    .all(|s| s.len() <= 4096),
            "retained stock reason cache extent differs"
        );
        ensure!(
            self.groups.0.iter().all(|g| g.guids.len() <= 65536),
            "retained Move group extent differs"
        );
        Ok(())
    }
}
