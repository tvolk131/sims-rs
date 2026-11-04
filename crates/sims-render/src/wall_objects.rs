//! Original4aa446..4aa54c temporary object hiding. The native renderer restores
//! the object field after drawing; here the decision never writes that field.
use anyhow::Result;
use sims_sim::lot_presenter::{lifecycle, walls::cutaway};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct View {
    pub rotation: u8,
    pub level: i32,
    pub all_walls: bool,
    pub cutaway: bool,
    pub mask: cutaway::Mask,
}
impl View {
    pub fn new(view: &lifecycle::State, rotation: u8) -> Self {
        Self {
            rotation,
            level: view.level,
            all_walls: view.all_walls != 0,
            cutaway: view.visibility.enabled != 0,
            mask: view.visibility.mask.clone(),
        }
    }
    pub fn hidden(
        &self,
        tile: [i8; 3],
        flags: u16,
        placement: u16,
        facing: i16,
        wall_tool: bool,
    ) -> Result<bool> {
        sims_sim::lot_presenter::object_cutaway::hidden(
            self.rotation,
            self.level,
            self.all_walls,
            self.cutaway,
            &self.mask,
            tile,
            flags,
            placement,
            facing,
            wall_tool,
        )
        .map_err(anyhow::Error::msg)
    }
}

/// 4358c3 and6f4670: front/back requirements, rotated by facing then world.
pub(crate) fn normal(placement: u16, facing: i16, rotation: u8) -> u8 {
    sims_sim::lot_presenter::object_cutaway::normal(placement, facing, rotation)
}

pub(crate) fn rotate([x, y, z]: [i8; 3], rotation: u8) -> [i8; 3] {
    match rotation {
        1 => [63i8.wrapping_sub(y), x, z],
        2 => [63i8.wrapping_sub(x), 63i8.wrapping_sub(y), z],
        3 => [y, 63i8.wrapping_sub(x), z],
        _ => [x, y, z],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn temporary_hiding_matches_unchanged_original_instructions() -> Result<()> {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/object-cutaway.json"))?;
        for (i, c) in report["cases"].as_array().unwrap().iter().enumerate() {
            let v = &c["input"];
            let mut view = View {
                rotation: v["rotation"].as_u64().unwrap() as u8,
                level: v["visible_level"].as_i64().unwrap() as i32,
                all_walls: v["all_walls"] == 1,
                cutaway: v["cutaway"] == 1,
                mask: cutaway::Mask {
                    first_row: 0,
                    rows: vec![0; 64],
                },
            };
            for p in v["mask"].as_array().unwrap() {
                view.mask.rows[p[1].as_u64().unwrap() as usize] |= 1 << p[0].as_u64().unwrap();
            }
            let hidden = view.hidden(
                serde_json::from_value(v["tile"].clone())?,
                v["flags"].as_u64().unwrap() as u16,
                v["placement"].as_u64().unwrap() as u16,
                v["facing"].as_i64().unwrap() as i16,
                v["wall_tool"] == 1,
            )?;
            assert_eq!(
                hidden,
                c["hidden"].as_bool().unwrap(),
                "original case{i}: {v}"
            );
        }
        Ok(())
    }
}
