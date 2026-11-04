//! 4cf48c/50f/592..672 and4cf4c4/547/6aa..7d6. Preserve the live
//! CPState prefix and the ordered4cef9b(7,-1) child before final dirty400.
use super::Fields;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Row,
    Column,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub index: i32,
    pub argument: u32,
}
const RESET: Request = Request {
    index: 7,
    argument: u32::MAX,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Fields,
    Tool,
    Dirty,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    catalog: u8,
    axis: Axis,
    value: i32,
    stage: Stage,
}
impl Change {
    pub fn new(catalog: u8, axis: Axis, value: i32) -> Result<Self, String> {
        if catalog > 6 {
            return Err("Buy category setter absent".into());
        }
        Ok(Self {
            catalog,
            axis,
            value,
            stage: Stage::Fields,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.catalog <= 6 {
            Ok(())
        } else {
            Err("Buy category setter absent".into())
        }
    }
    pub fn request(&self) -> Option<Request> {
        (self.stage == Stage::Tool).then_some(RESET)
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.request() != Some(request) {
            return Err("Buy category lost tool child".into());
        }
        self.stage = Stage::Dirty;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        fields.validate()?;
        match self.stage {
            Stage::Fields => {
                let c = self.catalog as usize;
                let axis = usize::from(self.axis == Axis::Column);
                if fields.indices[c][axis] == self.value {
                    self.stage = Stage::Complete;
                    return Ok(None);
                }
                if self.axis == Axis::Column {
                    fields.remember_page(c)?;
                } else {
                    fields.indices[c][1] = 0;
                }
                fields.indices[c][axis] = self.value;
                fields.reset_selection()?;
                self.stage = Stage::Tool;
            }
            Stage::Dirty => {
                fields.dirty |= 0x400;
                self.stage = Stage::Complete;
            }
            _ => (),
        }
        Ok(self.request())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::state::State;

    #[test]
    fn column_saves_page_and_resets_product_before_tool_then_reasserts_dirty() {
        for catalog in 0..7 {
            let mut f = State::constructed(1).buy;
            f.catalog = catalog;
            f.subcatalog = 1;
            f.indices[catalog as usize] = [2, 3];
            f.selected_table = 12;
            f.selected_object = 999;
            f.selection_aux = 7;
            f.cells[2 * 8 + 4] = 5;
            f.dirty = 8;
            let mut c = Change::new(catalog as u8, Axis::Column, 4).unwrap();
            assert_eq!(c.advance(&mut f).unwrap(), Some(RESET));
            assert_eq!(f.cells[2 * 8 + 3], 12);
            assert_eq!(
                (
                    f.selected_table,
                    f.selected_object,
                    f.selection_aux,
                    f.dirty
                ),
                (5, 0, 0, 0x408)
            );
            let prefix = f.clone();
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            assert_eq!(c.advance(&mut f).unwrap(), Some(RESET));
            assert_eq!(f, prefix); // waiting cannot repeat prefix/cache writes
            assert!(
                c.acknowledge(Request {
                    index: 6,
                    argument: u32::MAX
                })
                .is_err()
            );
            f.dirty = 0x80;
            f.selected_object = 321; // actual child may write CPState
            c.acknowledge(RESET).unwrap();
            assert_eq!(c.advance(&mut f).unwrap(), None);
            assert!(c.complete());
            assert_eq!((f.dirty, f.selected_object), (0x480, 321));
        }
    }
    #[test]
    fn equal_category_is_noop_but_subcatalog_and_page_calls_are_not() {
        let mut f = State::constructed(1).buy;
        f.subcatalog = 1;
        f.selected_table = 9;
        f.selected_object = 42;
        f.selection_aux = 3;
        f.dirty = 0;
        let before = f.clone();
        let mut c = Change::new(1, Axis::Row, 0).unwrap();
        assert_eq!(c.advance(&mut f).unwrap(), None);
        assert!(c.complete());
        assert_eq!(f, before);
        f.set_page(9).unwrap();
        assert_eq!(
            (f.cells[0], f.selected_object, f.selection_aux, f.dirty),
            (9, 42, 3, 0x400)
        );
        f.set_subcatalog(1).unwrap();
        assert_eq!(
            (f.selected_table, f.selected_object, f.selection_aux),
            (9, 0, 0)
        );
        f.catalog = 9;
        f.cells.fill(123);
        f.selected_object = 8;
        f.set_page(17).unwrap();
        assert!(f.cells.iter().all(|v| *v == 123));
        assert_eq!((f.selected_table, f.selected_object), (17, 8));
    }
    #[test]
    fn row_does_not_save_old_page_and_uses_active_catalog_after_prefix_writes() {
        let mut f = State::constructed(1).buy;
        f.catalog = 0;
        f.subcatalog = 1;
        f.indices[0] = [1, 2];
        f.indices[1] = [3, 4];
        f.cells[10] = 7;
        f.cells[24] = 8;
        f.selected_table = 99;
        let mut c = Change::new(1, Axis::Row, 5).unwrap();
        assert_eq!(c.advance(&mut f).unwrap(), Some(RESET));
        assert_eq!(f.indices[1], [5, 0]);
        assert_eq!(f.cells[10], 7);
        assert_eq!(f.cells[28], 0);
        // Each specific setter changes its own pair, but4cf3da uses+a0.
        assert_eq!(f.selected_table, 7);
    }
}
