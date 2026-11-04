//! Native 4a955d tile search and the terrain branch of 4a97c3 (flag one).
//! Camera fields are explicit. The DirectDraw depth-buffer path is separate.
use super::room_masks::View;

fn ratio(a: [i16; 2], b: [i16; 2]) -> Result<i32, String> {
    (i32::from(a[0]) - i32::from(b[0]))
        .wrapping_shl(16)
        .checked_div(i32::from(a[1]) - i32::from(b[1]))
        .ok_or_else(|| "native inverse projection integer division fault".into())
}
fn budget(remaining: &mut usize) -> Result<(), String> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or("native inverse projection search budget exhausted")?;
    Ok(())
}
fn integer(value: f64) -> Result<i32, String> {
    if !value.is_finite() || !(-9223372036854775808.0..9223372036854775808.0).contains(&value) {
        return Err("native inverse projection integer conversion unavailable".into());
    }
    Ok(value.trunc() as i64 as i32)
}
impl View<'_> {
    /// 4a955d. Input is vertical-first screen shorts; result is tile [x, y].
    pub fn coarse_point(&self, screen: [i16; 2], level: i32) -> Result<[i32; 2], String> {
        self.validate()?;
        let x = i32::from(screen[1].wrapping_sub(self.camera.origin[0] as i16)) / 2;
        let y = i32::from(screen[0].wrapping_sub(self.camera.origin[1] as i16));
        let shift = self.camera.zoom + 3;
        let mut best = [(y + x) >> shift, (y - x) >> shift];
        let project = |p: [i32; 2]| self.point([p[0] as i8, p[1] as i8, level as i8]);
        let mut parity = usize::from(project(best)?[1] >= screen[1]);
        let mut scan = best;
        let mut remaining = 65536;
        while scan[0] < 0 || scan[1] < 0 {
            budget(&mut remaining)?;
            scan[parity & 1] = scan[parity & 1].wrapping_add(1);
            parity = parity.wrapping_add(1);
        }
        let upper = [self.bounds[1] + 1, self.bounds[3] + 1];
        if scan[0] < upper[0] && scan[1] < upper[1] {
            loop {
                budget(&mut remaining)?;
                let axis = parity & 1;
                parity = parity.wrapping_add(1);
                scan[axis] = scan[axis].wrapping_add(1);
                if scan[axis] >= upper[axis] {
                    break;
                }
                if project(scan)?[0] < screen[0] {
                    best = scan;
                }
            }
        }
        loop {
            budget(&mut remaining)?;
            let p = project(best)?;
            if screen[1] < p[1] {
                let next = project([best[0], best[1].wrapping_add(1)])?;
                // Preserve original IDIV order, including its error boundary.
                let edge = ratio(p, next)?;
                if ratio(p, screen)? <= edge {
                    break;
                }
                best[0] = best[0].wrapping_sub(1);
            } else if screen[1] == p[1] {
                break;
            } else {
                let next = project([best[0].wrapping_add(1), best[1]])?;
                let edge = ratio(p, next)?;
                if edge <= ratio(p, screen)? {
                    break;
                }
                best[1] = best[1].wrapping_sub(1);
            }
        }
        Ok(best)
    }

    /// 4a97c3 with flag one: [fine y, fine x], the native physical-point order.
    /// Explicit f32 spills follow the original x87 instruction sequence.
    pub fn fine_point(&self, screen: [i16; 2], level: i32) -> Result<[i32; 2], String> {
        let tile = self.coarse_point(screen, level)?;
        let mut result = [tile[1].wrapping_shl(4), tile[0].wrapping_shl(4)];
        let p = [tile[0] as i8, tile[1] as i8, 1];
        let [x, y, _] = p.map(i32::from);
        if x <= self.bounds[0] || x >= self.bounds[1] || y <= self.bounds[2] || y >= self.bounds[3]
        {
            return Ok(result);
        }
        let h = self.terrain(p)?.map(i32::from);
        let center = self.point(p)?;
        let left = screen[1] < center[1];
        let width = self.camera.half_width;
        let side_x = if left {
            center[1].wrapping_sub(width as i16)
        } else {
            center[1].wrapping_add(width as i16)
        };
        let side_y = center[0].wrapping_add(
            ((h[1] - h[if left { 0 } else { 2 }] + 2) << (self.camera.zoom + 1)) as i16,
        );
        let bottom_y = center[0].wrapping_add(((h[1] - h[3] + 4) << (self.camera.zoom + 1)) as i16);
        let denominator = f64::from(i32::from(center[1]) - i32::from(side_x));
        let first = f64::from(i32::from(center[0]) - i32::from(side_y)) / denominator;
        let second = f64::from(i32::from(bottom_y) - i32::from(side_y)) / denominator;
        let shifted_y = screen[0].wrapping_add(
            self.camera.floor_heights[self.camera.zoom as usize].wrapping_mul(level.wrapping_sub(1))
                as i16,
        );
        let mouse_x = f64::from(screen[1]);
        let side_x = f64::from(side_x);
        let side_y = f64::from(side_y);
        let (stored, retained) = if first == second || first.is_nan() || second.is_nan() {
            (mouse_x, mouse_x)
        } else {
            let delta = second - first;
            (
                f64::from(
                    ((side_y - f64::from(shifted_y) - side_x * first + mouse_x * second) / delta)
                        as f32,
                ),
                (f64::from(shifted_y) - side_y - mouse_x * first + side_x * second) / delta,
            )
        };
        let center_x = f64::from(center[1]);
        let (a, b) = if left {
            (
                (center_x - stored) / f64::from(width) * 16.0,
                (retained - side_x) / f64::from(width) * 16.0,
            )
        } else {
            let width = f64::from(width as f32);
            (
                (side_x - retained) / width * 16.0,
                (stored - center_x) / width * 16.0,
            )
        };
        result[0] |= integer(a)? & 15;
        result[1] |= integer(b)? & 15;
        Ok(result)
    }
}
