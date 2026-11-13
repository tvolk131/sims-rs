//! Native4b0615 terrain extrema,4a9fa9 bounds and4a81cc scroll ownership.
//! Renderer synchronization is an acknowledged service; no Windows surface or
//! automatic viewer dispatch is supplied by this module.
use super::room_masks::{self, Camera};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extrema {
    pub maximum: Vec<u8>,
    pub minimum: Vec<u8>,
}
/// Native4b0615 indexes each diagonal by x+y after adding the lot origin.
pub fn terrain_extrema(
    size: i32,
    origin: [i32; 2],
    rotation: u8,
    heights: &[[u8; 4]],
) -> Result<Extrema, String> {
    if !(1..=64).contains(&size) || rotation > 3 || heights.len() != 4096 {
        return Err("native camera terrain input unavailable".into());
    }
    let mut result = Extrema {
        maximum: vec![0; 128],
        minimum: vec![64; 128],
    };
    for y in 0..size {
        for x in 0..size {
            let p = [
                (x as i8).wrapping_add(origin[0] as i8),
                (y as i8).wrapping_add(origin[1] as i8),
                0,
            ];
            if p[..2].iter().any(|&v| !(0..64).contains(&v)) {
                return Err("native camera terrain point outside rotation table".into());
            }
            let [a, b, _] = room_masks::rotate(p, 4u8.wrapping_sub(rotation) & 3);
            // All four rotated corners contribute, so their permutation does
            // not change either extremum. The initial minimum is exactly64.
            for h in heights[a as usize * 64 + b as usize] {
                let i = (i32::from(p[0]) + i32::from(p[1])) as usize;
                result.maximum[i] = result.maximum[i].max(h);
                result.minimum[i] = result.minimum[i].min(h);
            }
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub camera: Camera,
    pub size: i32,
    /// View+28,+2c,+30,+34, narrowed to signed shorts by4a9fa9.
    pub screen: [i32; 4],
    pub lot_origin: [i32; 2],
    /// View+64, separate from the camera's other width fields.
    pub row_step: i32,
    /// None represents the original null cache branches.
    pub maximum: Option<Vec<u8>>,
    pub minimum: Option<Vec<u8>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: [i32; 2],
    pub y: [i32; 2],
    /// View+1f4 publication, not a clamped viewport coordinate.
    pub bottom_margin: i32,
}
fn cached(cache: &[u8], p: [i8; 2]) -> Result<i32, String> {
    let i = i32::from(p[0]) + i32::from(p[1]);
    usize::try_from(i)
        .ok()
        .and_then(|i| cache.get(i))
        .copied()
        .map(i32::from)
        .ok_or_else(|| "native camera height-cache entry unavailable".into())
}
pub fn bounds(c: &Context) -> Result<Bounds, String> {
    if !(1..=64).contains(&c.size)
        || c.camera.zoom > 3
        || c.camera.floor_heights.len() <= c.camera.zoom as usize
    {
        return Err("native camera bounds fields unavailable".into());
    }
    let camera = &c.camera;
    let [left, _, right, bottom] = c.screen.map(|v| i32::from(v as i16));
    let width = right.wrapping_sub(left);
    let span = (c.size - 2).wrapping_mul(camera.tile_width);
    let mut x = if width < span {
        [
            (span / -2)
                .wrapping_sub(camera.half_width)
                .wrapping_add(right),
            (span / 2).wrapping_add(camera.tile_width),
        ]
    } else {
        [width / 2; 2]
    };
    let origin = c.lot_origin.map(|n| n as i8);
    let shift = u32::from(camera.zoom) + 1;
    let high = if let Some(cache) = &c.maximum {
        let mut p = origin.map(|n| n.wrapping_add(1));
        let mut high = 0;
        for n in 1..c.size - 1 {
            high = high.max(
                cached(cache, p)?
                    .wrapping_shl(shift)
                    .wrapping_sub(c.row_step.wrapping_mul(n)),
            );
            p = p.map(|n| n.wrapping_add(1));
        }
        high
    } else {
        64i32.wrapping_shl(shift)
    };
    let low = if let Some(cache) = &c.minimum {
        let mut p = origin.map(|n| n.wrapping_add((c.size - 2) as i8));
        let mut low = i32::MAX;
        let mut step = c.row_step;
        for _ in 1..c.size - 1 {
            low = low.min(step.wrapping_sub(cached(cache, p)?.wrapping_shl(shift)));
            step = step.wrapping_add(c.row_step);
            p = p.map(|n| n.wrapping_sub(1));
        }
        low
    } else {
        0
    };
    let mut y = [
        bottom
            .wrapping_sub(camera.bottom.wrapping_mul(2))
            .wrapping_sub((c.size - 2).wrapping_mul(camera.bottom))
            .wrapping_sub(low),
        high.wrapping_add(camera.floor_heights[camera.zoom as usize].wrapping_mul(2)),
    ];
    let [a, b] = origin.map(i32::from);
    let dx = a.wrapping_sub(b).wrapping_mul(camera.half_width);
    let dy = a.wrapping_add(b).wrapping_mul(c.row_step);
    x = x.map(|v| v.wrapping_sub(dx));
    y = y.map(|v| v.wrapping_sub(dy));
    Ok(Bounds {
        x,
        y,
        bottom_margin: low,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub origin: [i32; 2],
    pub pending: [i32; 2],
    pub deferred: u8,
    pub locked: u8,
    pub attempts: u32,
    pub bottom_margin: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synchronize {
    pub zoom: u8,
    pub origin: [i32; 2],
    pub rotation: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    request: Option<Synchronize>,
}
impl Cursor {
    pub fn begin(
        state: &mut State,
        context: &Context,
        target: [i32; 2],
        immediate: u8,
        renderer_ready: bool,
        rotation: u32,
    ) -> Result<Self, String> {
        state.attempts = state.attempts.wrapping_add(1);
        let done = Self { request: None };
        if state.locked != 0 {
            return Ok(done);
        }
        let b = bounds(context)?;
        state.bottom_margin = b.bottom_margin;
        // Ordered lower/upper comparisons also preserve reversed bounds.
        let target = [
            target[0].max(b.x[0]).min(b.x[1]).wrapping_add(1) & !3,
            target[1].max(b.y[0]).min(b.y[1]),
        ];
        if state.deferred != 0 {
            for n in 0..2 {
                state.pending[n] =
                    state.pending[n].wrapping_add(target[n].wrapping_sub(state.origin[n]));
            }
            if immediate == 0 || state.pending == [0, 0] {
                return Ok(done);
            }
            for n in 0..2 {
                state.origin[n] = state.origin[n].wrapping_add(state.pending[n]);
            }
            state.pending = [0, 0];
        } else {
            state.origin = target;
        }
        Ok(Self {
            request: renderer_ready.then_some(Synchronize {
                zoom: context.camera.zoom,
                origin: state.origin,
                rotation,
            }),
        })
    }
    pub fn request(&self) -> Option<&Synchronize> {
        self.request.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.request.is_none()
    }
    pub fn acknowledge(&mut self, request: &Synchronize) -> Result<(), String> {
        if self.request.as_ref() != Some(request) {
            return Err("native camera synchronization acknowledgement mismatch".into());
        }
        self.request = None;
        Ok(())
    }
}
