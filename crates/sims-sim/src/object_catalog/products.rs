//! Definition-backed Buy products: 4cd149 eligibility, 4cbd44 price, and
//! 507288 / 4cb8e7..4cbade category predicates. No UI or resource ownership.
use serde::{Deserialize, Serialize};
pub mod categories;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Excluded,
    Included,
    /// 4575a1 asks 457cb0 for a nonempty TTAB (not a drawing/sprite group).
    Interactions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub kind: i16,
    pub group: i16,
    pub subindex: i16,
    pub graphic: i16,
    pub interactions: i16,
    pub strings: u16,
    pub price: i32,
    pub room: i16,
    pub function: i16,
    pub subfunction: i16,
    pub community: [i16; 5],
}
impl Definition {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() != 216 {
            return Err("Buy product requires normalized OBJD".into());
        }
        let word = |n| i16::from_le_bytes(data[n..n + 2].try_into().unwrap());
        Ok(Self {
            kind: word(0x12),
            group: word(0x14),
            subindex: word(0x16),
            graphic: word(8),
            interactions: word(0xe),
            strings: word(0x52) as u16,
            price: i32::from(word(0x24)),
            room: word(0x4e),
            function: word(0x50),
            subfunction: word(0xbc),
            community: [0xbe, 0xc2, 0xc6, 0xce, 0xd0].map(word),
        })
    }
    pub fn eligibility(self) -> Eligibility {
        if (self.group != 0 && self.subindex != -1) || self.kind == 2 {
            Eligibility::Excluded
        } else if self.group != 0 || self.graphic != 0 {
            Eligibility::Included
        } else {
            Eligibility::Interactions
        }
    }
    /// The first predicate used when enumerating products, before row/column.
    pub fn present(self, catalog: i32) -> bool {
        match catalog {
            0 | 1 => self.room != 0,
            2..=6 => self.community[catalog as usize - 2] != 0,
            _ => false,
        }
    }
    /// Also used for previously visible products, without the presence gate.
    pub fn matches(self, catalog: i32, masks: Masks) -> bool {
        let intersects = |word: i16, mask| (i32::from(word) as u32 & mask) != 0;
        let row = match catalog {
            0 => self.room,
            1 => {
                return intersects(self.function, masks.row)
                    && (intersects(self.subfunction, masks.column)
                        || (masks.column == 0xff && self.subfunction == 0));
            }
            2..=6 => self.community[catalog as usize - 2],
            _ => return false,
        };
        intersects(row, masks.row) && intersects(self.function, masks.column)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Masks {
    pub row: u32,
    pub column: u32,
}
impl Masks {
    /// Unknown catalogs retain the previous masks. Invalid signed row/column
    /// indices in a known catalog produce zero, as the native switches do.
    pub fn update(&mut self, catalog: i32, indices: &[[i32; 2]; 7]) {
        const ROOM: [u32; 8] = [8, 32, 2, 128, 1, 4, 16, 64];
        const FUNCTION: [u32; 8] = [1, 2, 32, 8, 4, 16, 128, 64];
        const SUBFUNCTION: [u32; 8] = [1, 2, 4, 8, 32, 64, 0x90, 0xff];
        const COMMUNITY: [u32; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
        if !(0..=6).contains(&catalog) {
            return;
        }
        let [row, column] = indices[catalog as usize];
        let value = |table: &[u32; 8], i: i32| table.get(i as usize).copied().unwrap_or(0);
        self.row = value(
            match catalog {
                0 => &ROOM,
                1 => &FUNCTION,
                _ => &COMMUNITY,
            },
            row,
        );
        self.column = value(
            if catalog == 1 {
                &SUBFUNCTION
            } else {
                &FUNCTION
            },
            column,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data(words: &[(usize, i16)]) -> Vec<u8> {
        let mut data = vec![0; 216];
        for &(n, word) in words {
            data[n..n + 2].copy_from_slice(&word.to_le_bytes());
        }
        data
    }
    #[test]
    fn membership_excludes_people_and_parts_before_graphics_or_interactions() {
        for kind in [0, 2, 4] {
            for group in [0, -1, 7] {
                for subindex in [-1, 0, 1] {
                    for graphic in [-1, 0, 1] {
                        let d = Definition::parse(&data(&[
                            (0x12, kind),
                            (0x14, group),
                            (0x16, subindex),
                            (8, graphic),
                        ]))
                        .unwrap();
                        assert_eq!(
                            d.eligibility(),
                            if kind == 2 || (group != 0 && subindex != -1) {
                                Eligibility::Excluded
                            } else if group != 0 || graphic != 0 {
                                Eligibility::Included
                            } else {
                                Eligibility::Interactions
                            }
                        );
                    }
                }
            }
        }
        assert_eq!(
            Definition::parse(&data(&[(0x24, -32768)])).unwrap().price,
            -32768
        );
        assert!(Definition::parse(&[0; 214]).is_err());
    }
    #[test]
    fn categories_keep_signed_masks_and_the_miscellaneous_zero_fallback() {
        let mut d = Definition::parse(&data(&[(0x4e, 1), (0x50, -32768), (0xbc, 0)])).unwrap();
        assert!(d.matches(
            1,
            Masks {
                row: 0xffff0000,
                column: 0xff
            }
        ));
        assert!(!d.matches(
            1,
            Masks {
                row: 0xffff0000,
                column: 0x90
            }
        ));
        d.subfunction = 16;
        assert!(d.matches(
            1,
            Masks {
                row: 0x8000,
                column: 0x90
            }
        ));
        assert!(!d.matches(
            1,
            Masks {
                row: 0x8000,
                column: 32
            }
        ));
        d.room = 0;
        assert!(d.matches(
            1,
            Masks {
                row: 0x8000,
                column: 0xff
            }
        ));
        assert!(!d.present(1));
        for c in 2..=6 {
            d.community = [0; 5];
            d.community[c as usize - 2] = -1;
            assert!(d.present(c));
            assert!(d.matches(
                c,
                Masks {
                    row: 0x80000000,
                    column: 0x8000
                }
            ));
            assert!(!d.present(if c == 6 { 2 } else { c + 1 }));
        }
    }
    #[test]
    fn native_switch_tables_and_invalid_indices() {
        let expected = [
            ([8, 32, 2, 128, 1, 4, 16, 64], [1, 2, 32, 8, 4, 16, 128, 64]),
            (
                [1, 2, 32, 8, 4, 16, 128, 64],
                [1, 2, 4, 8, 32, 64, 144, 255],
            ),
            ([1, 2, 4, 8, 16, 32, 64, 128], [1, 2, 32, 8, 4, 16, 128, 64]),
        ];
        for catalog in 0..7 {
            let (rows, columns) = expected[catalog.min(2)];
            for i in -1..=8 {
                let mut masks = Masks {
                    row: 123,
                    column: 456,
                };
                masks.update(catalog as i32, &[[i, i]; 7]);
                assert_eq!(
                    masks,
                    Masks {
                        row: rows.get(i as usize).copied().unwrap_or(0),
                        column: columns.get(i as usize).copied().unwrap_or(0)
                    }
                );
            }
        }
        let mut masks = Masks {
            row: 123,
            column: 456,
        };
        for catalog in [-1, 7, i32::MIN, i32::MAX] {
            masks.update(catalog, &[[0; 2]; 7]);
        }
        assert_eq!(
            masks,
            Masks {
                row: 123,
                column: 456
            }
        );
    }
}
