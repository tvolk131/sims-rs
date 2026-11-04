//! Same-zoom, no-height-override4a9dc7 fine-point projection used by centering.
use super::room_masks::View;
fn div(a: i32, b: i32) -> Result<i32, String> {
    a.checked_div(b)
        .ok_or_else(|| "native fine projection integer division fault".into())
}
impl View<'_> {
    /// Original4a7996 same-floor, graphics-ready, immediate centering target.
    /// None is the inclusive outside-lot rejection. The caller must send the
    /// target through native scrolling; floor changes and animated movement
    /// have separate ownership and are not implemented by this helper.
    pub fn centered_origin(
        &self,
        viewport: [i32; 4],
        pending: [i32; 2],
        physical: [i32; 2],
        level: i32,
    ) -> Result<Option<[i32; 2]>, String> {
        self.validate()?;
        let [x, y] = [physical[1] >> 4, physical[0] >> 4].map(|v| i32::from(v as i8));
        let b = self.bounds;
        if x < b[0] || x > b[1] || y < b[2] || y > b[3] {
            return Ok(None);
        }
        let [vertical, horizontal] = self.fine_screen(physical, level)?.map(i32::from);
        let half = [
            viewport[2].wrapping_sub(viewport[0]) / 2,
            viewport[3].wrapping_sub(viewport[1]) / 2,
        ];
        let screen = [horizontal, vertical];
        Ok(Some(std::array::from_fn(|n| {
            half[n]
                .wrapping_sub(screen[n])
                .wrapping_add(self.camera.origin[n])
                .wrapping_add(pending[n])
        })))
    }

    /// Native physical-point order [fine y,fine x]; screen is vertical first.
    pub fn fine_screen(&self, physical: [i32; 2], level: i32) -> Result<[i16; 2], String> {
        self.fine_screen_with_height(physical, level, None)
    }
    ///4a9dc7's optional height-byte pointer overrides all four terrain corners,
    /// including at borders. The current projection zoom remains unchanged.
    pub fn fine_screen_with_height(
        &self,
        physical: [i32; 2],
        level: i32,
        height: Option<u8>,
    ) -> Result<[i16; 2], String> {
        self.validate()?;
        let p = [
            (physical[1] >> 4) as i8,
            (physical[0] >> 4) as i8,
            level as i8,
        ];
        let mut screen = if let Some(h) = height {
            let [x, y] = self.project(p);
            [
                y.wrapping_sub(i32::from(h).wrapping_shl(u32::from(self.camera.zoom) + 1)) as i16,
                x as i16,
            ]
        } else {
            self.point(p)?
        };
        let [x, y, _] = p.map(i32::from);
        let h = if let Some(h) = height {
            [i32::from(h); 4]
        } else {
            if x > self.bounds[0] && x < self.bounds[1] && y > self.bounds[2] && y < self.bounds[3]
            {
                self.terrain(p)?.map(i32::from)
            } else {
                [0; 4]
            }
        };
        let u = physical[1] & 15;
        let v = physical[0] & 15;
        let shift = u32::from(self.camera.zoom) + 1;
        let diagonal = |n: i32| div((h[1] - h[3] + 4).wrapping_shl(shift).wrapping_mul(n), 16);
        let (dy, dx) = if u == v {
            (diagonal(u)?, 0)
        } else {
            let (large, small, corner, denominator) =
                if v < u { (u, v, 2, 16) } else { (v, u, 0, -16) };
            let side = div(
                (h[1] - h[corner] + 2)
                    .wrapping_shl(shift)
                    .wrapping_mul(large),
                16,
            )?;
            let horizontal = div(self.camera.half_width.wrapping_mul(large), denominator)?;
            (
                div(
                    diagonal(large)?.wrapping_sub(side).wrapping_mul(small),
                    large,
                )?
                .wrapping_add(side),
                horizontal.wrapping_sub(div(horizontal.wrapping_mul(small), large)?),
            )
        };
        screen[0] = screen[0].wrapping_add(dy as i16);
        screen[1] = screen[1].wrapping_add(dx as i16);
        Ok(screen)
    }
}
