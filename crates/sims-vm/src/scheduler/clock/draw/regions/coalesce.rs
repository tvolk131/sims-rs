//!5ccfe0's ordered overlap decisions. Integer costs below2^24 have the same
//! value at every native x87 spill/operation, independent of precision control.
//! Other numeric domains and vector growth retain an explicit child boundary.
use super::*;

fn valid(r: Rect) -> bool {
    r[0] < r[2] && r[1] < r[3]
}
fn contains(a: Rect, b: Rect) -> bool {
    a[0] <= b[0] && a[1] <= b[1] && a[2] >= b[2] && a[3] >= b[3]
}
fn union(a: Rect, b: Rect) -> Rect {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}
fn exact_area(r: Rect) -> Option<i64> {
    let v = i64::from(r[2].checked_sub(r[0])?) * i64::from(r[3].checked_sub(r[1])?);
    (0..(1 << 24)).contains(&v).then_some(v)
}
fn cost(rectangles: &[Rect], weight: i64) -> Option<i64> {
    let mut v = 0i64;
    let mut bound = 0i64;
    for &r in rectangles {
        let area = exact_area(r)?;
        // Every native addition order must remain exact, including a
        // negative weight cancelling a larger positive intermediate sum.
        bound += area + weight.abs();
        if bound >= 1 << 24 {
            return None;
        }
        v += area + weight;
    }
    Some(v)
}
fn mask(a: Rect, b: Rect) -> u8 {
    let inside = |x, y| a[0] <= x && x <= a[2] && a[1] <= y && y <= a[3];
    u8::from(inside(b[0], b[1])) * 8
        | u8::from(inside(b[2], b[1])) * 4
        | u8::from(inside(b[0], b[3])) * 2
        | u8::from(inside(b[2], b[3]))
}
#[derive(Clone)]
struct Choice {
    first: Rect,
    second: Option<Rect>,
    append: Vec<Rect>,
}
impl Choice {
    fn pair(first: Rect, second: Rect) -> Self {
        Self {
            first,
            second: Some(second),
            append: vec![],
        }
    }
    fn merged(first: Rect) -> Self {
        Self {
            first,
            second: None,
            append: vec![],
        }
    }
    fn split(first: Rect, second: Rect, append: Vec<Rect>) -> Self {
        Self {
            first,
            second: Some(second),
            append,
        }
    }
    fn cost(&self, weight: i64) -> Option<i64> {
        let mut rectangles = vec![self.first];
        rectangles.extend(self.second);
        rectangles.extend(&self.append);
        cost(&rectangles, weight)
    }
}
fn cheapest(choices: &[Choice], weight: i64) -> Option<Choice> {
    let mut best = None;
    for c in choices {
        let v = c.cost(weight)?;
        if best.as_ref().is_none_or(|(value, _)| v < *value) {
            best = Some((v, c.clone()));
        }
    }
    best.map(|(_, c)| c)
}
impl List {
    /// Full ordered rectangle scan for the exact integer-cost domain. False
    /// leaves all header/backing bytes unchanged, including unused capacity.
    pub fn coalesce(&mut self, enabled: u32, weight: f32) -> Result<bool, String> {
        self.validate()?;
        if enabled == 0 || self.header[12] != 0 || self.len() < 2 {
            return self.coalesce_separated(enabled, weight);
        }
        // Keep the already qualified separated/floating-weight path.
        let mut separated = self.clone();
        if separated.coalesce_separated(enabled, weight)? {
            *self = separated;
            return Ok(true);
        }
        if !weight.is_finite() || weight.fract() != 0. || weight.abs() >= (1 << 22) as f32 {
            return Ok(false);
        }
        let mut work = self.clone();
        if !work.scan_overlaps(weight as i64)? || !work.finish_coalescing(weight)? {
            return Ok(false);
        }
        *self = work;
        Ok(true)
    }
    ///5ccf96 clips each record in place before ordered erasure/coalescing.
    pub fn clip(&mut self, rectangle: Rect, enabled: u32, weight: f32) -> Result<bool, String> {
        self.validate()?;
        if enabled == 0 {
            return Ok(true);
        }
        let mut work = self.clone();
        let mut i = 0;
        while i < work.len() {
            let mut r = work.rectangle(i)?;
            //4a76ed clamps each endpoint independently, retaining a boundary
            // point for an entirely disjoint rectangle before it is erased.
            r[0] = r[0].max(rectangle[0]).min(rectangle[2]);
            r[1] = r[1].max(rectangle[1]).min(rectangle[3]);
            r[2] = r[2].min(rectangle[2]).max(rectangle[0]);
            r[3] = r[3].min(rectangle[3]).max(rectangle[1]);
            work.write_rectangle(i, r);
            if valid(r) {
                i += 1;
            } else {
                work.erase(i)?;
            }
        }
        if !work.coalesce(enabled, weight)? {
            return Ok(false);
        }
        *self = work;
        Ok(true)
    }
    fn write_rectangle(&mut self, index: usize, rectangle: Rect) {
        for (n, v) in rectangle.into_iter().enumerate() {
            set(&mut self.backing, 16 * index + 4 * n, v as u32);
        }
    }
    fn apply_choice(&mut self, i: usize, j: usize, c: Choice) -> Result<bool, String> {
        self.write_rectangle(i, c.first);
        if let Some(r) = c.second {
            self.write_rectangle(j, r);
        } else {
            self.erase(j)?;
        }
        for r in c.append {
            if !self.append(r)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn scan_overlaps(&mut self, weight: i64) -> Result<bool, String> {
        // Reject overflow in523fbd's four signed, decremented differences.
        for i in 0..self.len() {
            let r = self.rectangle(i)?;
            if r.iter().any(|v| !(-1_000_000..=1_000_000).contains(v)) {
                return Ok(false);
            }
        }
        let mut restarts = 1;
        'restart: loop {
            let mut i = 0;
            while i + 1 < self.len() {
                let a = self.rectangle(i)?;
                if !valid(a) {
                    self.erase(i)?;
                    continue;
                }
                let mut j = i + 1;
                while j < self.len() {
                    let b = self.rectangle(j)?;
                    if !valid(b) || contains(a, b) {
                        self.erase(j)?;
                        continue;
                    }
                    if contains(b, a) {
                        self.write_rectangle(i, b);
                        self.erase(j)?;
                    } else {
                        if a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1] {
                            j += 1;
                            continue;
                        }
                        let m = mask(a, b);
                        let choice = if matches!(m, 1 | 2 | 4 | 8) {
                            let mut horizontal = b;
                            let mut vertical = b;
                            if matches!(m, 1 | 2) {
                                horizontal[3] = a[1];
                            } else {
                                horizontal[1] = a[3];
                            }
                            if matches!(m, 1 | 4) {
                                vertical[2] = a[0];
                            } else {
                                vertical[0] = a[2];
                            }
                            let mut rv = vertical;
                            rv[1] = rv[1].max(a[1]);
                            rv[3] = rv[3].min(a[3]);
                            let mut rh = horizontal;
                            rh[0] = rh[0].max(a[0]);
                            rh[2] = rh[2].min(a[2]);
                            let mut choices = vec![];
                            for (strip, remainder) in [(horizontal, rv), (vertical, rh)] {
                                // Native fills four sentinel costs for an empty
                                // remainder. Do not consider that candidate set.
                                if !valid(remainder) {
                                    continue;
                                }
                                choices.extend([
                                    Choice::pair(union(a, strip), remainder),
                                    Choice::merged(union(union(a, strip), remainder)),
                                    Choice::pair(union(a, remainder), strip),
                                    Choice::split(a, strip, vec![remainder]),
                                ]);
                            }
                            cheapest(&choices, weight)
                        } else if matches!(m, 3 | 5 | 10 | 12) {
                            let mut remaining = b;
                            match m {
                                3 => remaining[3] = a[1],
                                5 => remaining[2] = a[0],
                                10 => remaining[0] = a[2],
                                12 => remaining[1] = a[3],
                                _ => unreachable!(),
                            }
                            //5cd3c3 modifies the second record before its cost
                            // comparison, even when keeping both rectangles.
                            self.write_rectangle(j, remaining);
                            let Some(separate) = cost(&[a, remaining], weight) else {
                                return Ok(false);
                            };
                            let Some(merged) = cost(&[union(a, remaining)], weight) else {
                                return Ok(false);
                            };
                            if separate <= merged {
                                self.write_rectangle(j, remaining);
                                j += 1;
                                continue;
                            }
                            Some(Choice::merged(union(a, remaining)))
                        } else if m == 0 {
                            let in_x = |v| a[0] <= v && v <= a[2];
                            let in_y = |v| a[1] <= v && v <= a[3];
                            let cross = u8::from(in_x(b[0]) && b[1] <= a[1] && a[3] <= b[3]) * 8
                                | u8::from(in_x(b[2]) && b[1] <= a[1] && a[3] <= b[3]) * 4
                                | u8::from(b[0] <= a[0] && a[2] <= b[2] && in_y(b[1])) * 2
                                | u8::from(b[0] <= a[0] && a[2] <= b[2] && in_y(b[3]));
                            let (mut center, mut first, mut second) = (b, b, b);
                            match cross {
                                1 | 2 => {
                                    if cross == 1 {
                                        center[3] = a[1];
                                    } else {
                                        center[1] = a[3];
                                    }
                                    center[0] = a[0];
                                    center[2] = a[2];
                                    first[2] = a[0];
                                    second[0] = a[2];
                                }
                                4 | 8 => {
                                    if cross == 4 {
                                        center[2] = a[0];
                                    } else {
                                        center[0] = a[2];
                                    }
                                    center[1] = a[1];
                                    center[3] = a[3];
                                    first[3] = a[1];
                                    second[1] = a[3];
                                }
                                3 | 12 => {
                                    if cross == 3 {
                                        center[2] = a[0];
                                        first[0] = a[2];
                                    } else {
                                        center[3] = a[1];
                                        first[1] = a[3];
                                    }
                                    self.write_rectangle(j, center);
                                    let ua = union(a, center);
                                    let Some(c) = cheapest(
                                        &[
                                            Choice::merged(union(ua, first)),
                                            Choice::pair(ua, first),
                                            Choice::split(a, center, vec![first]),
                                        ],
                                        weight,
                                    ) else {
                                        return Ok(false);
                                    };
                                    let restart = c.first != a || c.second.is_none();
                                    if !self.apply_choice(i, j, c)? {
                                        return Ok(false);
                                    }
                                    if !restart {
                                        j += 1;
                                        continue;
                                    }
                                    restarts += 1;
                                    if restarts > 100 {
                                        return Ok(true);
                                    }
                                    continue 'restart;
                                }
                                _ => return Ok(false),
                            }
                            self.write_rectangle(j, center);
                            let ua = union(a, center);
                            let uaf = union(ua, first);
                            cheapest(
                                &[
                                    Choice::merged(union(uaf, second)),
                                    Choice::pair(uaf, second),
                                    Choice::split(ua, first, vec![second]),
                                    Choice::split(a, center, vec![first, second]),
                                ],
                                weight,
                            )
                        } else {
                            return Ok(false);
                        };
                        let Some(choice) = choice else {
                            return Ok(false);
                        };
                        let restart = choice.first != a || choice.second.is_none();
                        if !self.apply_choice(i, j, choice)? {
                            return Ok(false);
                        }
                        if !restart {
                            j += 1;
                            continue;
                        }
                    }
                    restarts += 1;
                    if restarts > 100 {
                        return Ok(true);
                    }
                    continue 'restart;
                }
                i += 1;
            }
            return Ok(true);
        }
    }
}
