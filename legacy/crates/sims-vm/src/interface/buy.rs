//! Native Buy catalog selection fields,4cf3da/4cf3af/4cfe1c. Content loading,
//! tool construction, purchasing and placement are separate owners.
use serde::{Deserialize, Serialize};
pub mod category;
pub mod definition;
pub mod input;
pub mod product;
pub mod tool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub dirty: u32,
    ///Interface+a0,+a4.
    pub catalog: i32,
    pub subcatalog: i32,
    ///Seven pairs at+a8..+dc; signed native row/column indices.
    pub indices: [[i32; 2]; 7],
    ///+ec..+1eb;64 remembered page numbers, shared by the seven catalogs.
    pub cells: Vec<u32>,
    ///+e0 current page number. Historical field/wire name retained.
    pub selected_table: u32,
    ///+e4 resident product identity; distinct from a definition or Sim ID.
    pub selected_object: u32,
    ///+e8 selected visible-product ordinal, or ffffffff for no selection.
    pub selection_aux: u32,
}
impl Fields {
    pub fn validate(&self) -> Result<(), String> {
        if self.cells.len() == 64 {
            Ok(())
        } else {
            Err("Buy catalog cache must contain64 cells".into())
        }
    }
    pub fn reset_selection(&mut self) -> Result<(), String> {
        self.validate()?;
        // These writes precede the native indexed read, even if that read falls
        // outside the bounded cache profile supported by this host.
        self.selection_aux = 0;
        self.selected_object = 0;
        if self.subcatalog == 0 {
            self.selected_table = 0;
        } else if (0..=6).contains(&self.catalog) {
            let [row, column] = self.indices[self.catalog as usize];
            let bytes = row.wrapping_mul(8).wrapping_add(column).wrapping_mul(4) as u32;
            self.selected_table = *self
                .cells
                .get((bytes / 4) as usize)
                .ok_or("native Buy catalog index is outside retained cache")?;
        }
        //Unknown catalog selectors leave+e0 alone. Only+e4/+e8 are reset.
        self.dirty |= 0x400;
        Ok(())
    }
    pub fn set_catalog(&mut self, value: i32) -> Result<(), String> {
        self.validate()?;
        self.subcatalog = 0;
        self.catalog = value;
        self.reset_selection()
    }
    ///4cf3c8 resets selection even if the subcatalog value is unchanged.
    pub fn set_subcatalog(&mut self, value: i32) -> Result<(), String> {
        self.validate()?;
        self.subcatalog = value;
        self.reset_selection()
    }
    pub(super) fn remember_page(&mut self, catalog: usize) -> Result<(), String> {
        let [row, column] = self.indices[catalog];
        let bytes = row.wrapping_mul(8).wrapping_add(column).wrapping_mul(4) as u32;
        let cell = self
            .cells
            .get_mut((bytes / 4) as usize)
            .ok_or("native Buy page cache index is outside retained cache")?;
        *cell = self.selected_table;
        Ok(())
    }
    ///4cf83f retains the outgoing page, marks400 and writes the new page.
    ///It does not clear the selected product or invoke the tool setter.
    pub fn set_page(&mut self, page: u32) -> Result<(), String> {
        self.validate()?;
        if (0..=6).contains(&self.catalog) {
            self.remember_page(self.catalog as usize)?;
        }
        self.dirty |= 0x400;
        self.selected_table = page;
        Ok(())
    }
    pub fn clear_cache(&mut self) -> Result<(), String> {
        self.validate()?;
        self.cells.fill(0);
        Ok(())
    }
    ///53a2c3's already-Buy branch. Catalogs3..6 still clear the cache without
    ///calling4cf3af; the later equal-mode4cea5d invocation remains a no-op.
    pub fn repeat_buy_button(&mut self) -> Result<(), String> {
        self.validate()?;
        match self.catalog {
            0 => self.set_catalog(1)?,
            1 => self.set_catalog(0)?,
            2 => self.set_catalog(2)?,
            _ => (),
        }
        self.clear_cache()
    }
}
