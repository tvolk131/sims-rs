//!4c4388's no-primary bounds path and49d248's rectangle expansion.
//! Overlapping raster work remains an explicit pending child.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Top,
    Bottom,
    Height,
    Clip,
    Complete,
    Raster,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    point: [i8; 2],
    screen: [i32; 2],
    corners: [u8; 4],
    zoom: u32,
    bounds: [i32; 4],
    stage: Stage,
    rectangle: Option<[i32; 4]>,
}
impl Frame {
    pub fn new(
        point: [i8; 2],
        screen: [i32; 2],
        corners: [u8; 4],
        zoom: u32,
        bounds: [i32; 4],
    ) -> Self {
        Self {
            point,
            screen,
            corners,
            zoom,
            bounds,
            stage: Stage::Entry,
            rectangle: None,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3).contains(&self.zoom)
            || self.rectangle.is_some() != !matches!(self.stage, Stage::Entry)
                && self.stage != Stage::Complete
        {
            return Err("terrain rectangle lifetime differs".into());
        }
        Ok(())
    }
    pub fn matches_arguments(
        &self,
        point: [i8; 2],
        screen: [i32; 2],
        corners: [u8; 4],
        zoom: u32,
        bounds: [i32; 4],
    ) -> bool {
        (
            self.point,
            self.screen,
            self.corners,
            self.zoom,
            self.bounds,
        ) == (point, screen, corners, zoom, bounds)
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn raster_pending(&self) -> bool {
        self.stage == Stage::Raster
    }
    pub fn rectangle(&self) -> Option<[i32; 4]> {
        self.rectangle
    }
    pub fn advance(&mut self, flags: [u8; 2], height: i32, clip: [i16; 4]) -> Result<bool, String> {
        self.validate()?;
        if self.complete() || self.raster_pending() {
            return Ok(false);
        }
        match self.stage {
            Stage::Entry => {
                let [x, y] = self.point.map(i32::from);
                let h = self.corners.map(i32::from);
                if x <= self.bounds[0]
                    || x >= self.bounds[1]
                    || y <= self.bounds[2]
                    || y >= self.bounds[3]
                    || h[3] > h[1] + 5
                {
                    self.stage = Stage::Complete;
                } else {
                    let [x, y] = self.screen;
                    let half = 8i32.wrapping_shl(self.zoom);
                    let shift = self.zoom.wrapping_add(1);
                    let mut top = y.wrapping_sub(h[1].wrapping_shl(shift));
                    let mut bottom = top;
                    for v in [
                        h[2].wrapping_sub(2),
                        h[0].wrapping_sub(2),
                        h[3].wrapping_sub(4),
                    ] {
                        let v = y.wrapping_sub(v.wrapping_shl(shift));
                        top = top.min(v);
                        bottom = bottom.max(v);
                    }
                    self.rectangle =
                        Some([x.wrapping_sub(half), top, x.wrapping_add(half), bottom]);
                    self.stage = Stage::Top;
                }
            }
            Stage::Top => {
                self.rectangle.as_mut().unwrap()[1] = self.rectangle.unwrap()[1].wrapping_sub(2);
                self.stage = Stage::Bottom;
            }
            Stage::Bottom => {
                self.rectangle.as_mut().unwrap()[3] = self.rectangle.unwrap()[3].wrapping_add(2);
                self.stage = Stage::Height;
            }
            Stage::Height => {
                if flags[0] != 0 {
                    let amount = if flags[1] != 0 { height } else { 0 };
                    self.rectangle.as_mut().unwrap()[1] =
                        self.rectangle.unwrap()[1].wrapping_sub(amount);
                }
                self.stage = Stage::Clip;
            }
            Stage::Clip => {
                let r = self.rectangle.unwrap();
                let c = clip.map(i32::from);
                self.stage = if c[0] < r[3] && r[0] < c[3] && c[1] < r[2] && r[1] < c[2] {
                    Stage::Raster
                } else {
                    Stage::Complete
                };
            }
            Stage::Complete | Stage::Raster => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn raster_spec(
        &self,
        height: i32,
        clip: [i16; 4],
        restrict_slope: bool,
        draw: bool,
    ) -> Result<Option<super::terrain_raster::Spec>, String> {
        if !self.raster_pending() {
            return Err("terrain raster has no rectangle caller".into());
        }
        let h = self.corners.map(i32::from);
        if !draw
            || (restrict_slope
                && [(h[1] - h[0]), (h[1] - h[2]), (h[3] - h[0]), (h[3] - h[2])]
                    .iter()
                    .any(|v| v.abs() > 5))
        {
            return Ok(None);
        }
        let mut r = self.rectangle.unwrap();
        r[1] = r[1].wrapping_sub(height);
        let c = clip.map(i32::from);
        let clipped = r[1] < c[0] || r[3] > c[2] || r[2] > c[3] || r[0] < c[1];
        Ok(Some(super::terrain_raster::Spec {
            left: r[0],
            bottom: self.screen[1].wrapping_add(4i32.wrapping_shl(self.zoom)),
            corners: self.corners,
            zoom: self.zoom,
            clip,
            clipped,
            rectangle: r,
        }))
    }
    pub fn finish_raster(&mut self) -> Result<(), String> {
        if !self.raster_pending() {
            return Err("terrain raster return has no caller".into());
        }
        self.stage = Stage::Complete;
        Ok(())
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4c4388 => self.stage == Stage::Entry,
            0x49d24c => self.stage == Stage::Top,
            0x4c4622 => self.complete(),
            _ => false,
        }
    }
}
