//! 5050e4 availability and 506765 category buttons. Constructor matrices use
//! the first matching function/room, not every category in an object's mask.
use super::Definition;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Availability(pub [[[bool; 8]; 8]; 7]);
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Button {
    pub visible: bool,
    pub enabled: bool,
    pub selected: bool,
}
impl Availability {
    pub fn add(&mut self, d: Definition) -> Result<(), String> {
        fn first(word: i16, bits: [u16; 8]) -> i32 {
            bits.iter()
                .position(|&bit| word as u16 & bit != 0)
                .map_or(-1, |i| i as i32)
        }
        let function = || first(d.function, [1, 2, 32, 8, 4, 16, 128, 64]);
        if d.room != 0 {
            let row = first(d.room, [8, 32, 2, 128, 1, 4, 16, 64]);
            self.mark(0, row, function())?;
        }
        for (i, bits) in d.community.into_iter().enumerate() {
            for row in [0, 1, 2, 3, 7] {
                if bits as u16 & (1 << row) != 0 {
                    self.mark(i + 2, row, function())?;
                }
            }
        }
        if d.function != 0 && d.subfunction != 0 {
            // 5071e5 maps both 0x10 and 0x80 to Miscellaneous (column 6).
            let column = first(d.subfunction, [1, 2, 4, 8, 32, 64, 16, 128]).min(6);
            self.mark(1, function(), column)?;
        }
        Ok(())
    }
    fn mark(&mut self, catalog: usize, row: i32, column: i32) -> Result<(), String> {
        // The original first-bit helpers return -1 for zero/no supported bits.
        // Shipped entries actually take this path (e.g. Treasure Hunt). Its
        // byte write can select the preceding row or neighboring matrix. Keep
        // those observable flags with bounded indexing, never a Rust pointer.
        // Native matrices occupy6f2c68..6f2e27 in reverse catalog order.
        let offset = (6 - catalog as i32) * 64 + row * 8 + column;
        if !(0..448).contains(&offset) {
            return Err("Buy availability write outside the seven owned matrices".into());
        }
        let offset = offset as usize;
        self.0[6 - offset / 64][offset % 64 / 8][offset % 8] = true;
        Ok(())
    }
    pub fn buttons(
        &self,
        catalog: i32,
        subcatalog: i32,
        indices: &[[i32; 2]; 7],
    ) -> Result<[Button; 8], String> {
        let catalog: usize = catalog
            .try_into()
            .ok()
            .filter(|&i| i < 7)
            .ok_or("Buy catalog unsupported")?;
        let [row, column] = indices[catalog];
        if subcatalog != 0 && !(0..8).contains(&row) {
            return Err("Buy category row outside availability matrix".into());
        }
        Ok(std::array::from_fn(|i| {
            let (visible, enabled) = if subcatalog == 0 {
                (catalog < 2 || !matches!(i, 4..=6), true)
            } else {
                let available = self.0[catalog][row as usize][i];
                if catalog == 1 {
                    match i {
                        4 | 5 => (row == 7, row != 7 || available),
                        7 => (true, true),
                        _ => (true, available),
                    }
                } else {
                    (true, available)
                }
            };
            Button {
                visible,
                enabled,
                selected: subcatalog != 0 && column == i as i32,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_bit_constructor_differs_from_product_filter_and_keeps_community_rows() {
        let mut d = Definition::parse(&[0; 216]).unwrap();
        d.room = 0xff;
        d.function = 0xff;
        d.subfunction = 128;
        d.community = [0xff; 5];
        let mut a = Availability::default();
        a.add(d).unwrap();
        assert_eq!(a.0[0].iter().flatten().filter(|&&b| b).count(), 1);
        assert!(a.0[0][0][0]);
        assert!(a.0[1][0][6]);
        for matrix in &a.0[2..] {
            for row in 0..8 {
                assert_eq!(matrix[row][0], matches!(row, 0..=3 | 7));
                assert!(matrix[row][1..].iter().all(|&b| !b));
            }
        }
        let buttons = a.buttons(1, 1, &[[0, 6]; 7]).unwrap();
        assert!(!buttons[4].visible && !buttons[5].visible);
        assert!(buttons[6].enabled && buttons[6].selected && buttons[7].enabled);
        assert!(a.buttons(1, 1, &[[-1, 0]; 7]).is_err());
        d.room = 64;
        d.function = 0;
        d.subfunction = 0;
        d.community = [0; 5];
        let mut a = Availability::default();
        a.add(d).unwrap();
        assert!(a.0[0][6][7]);
        d.community[0] = 1;
        a.add(d).unwrap();
        assert!(a.0[3][7][7]);
        d.community[4] = 1;
        assert!(a.add(d).is_err());
    }
}
