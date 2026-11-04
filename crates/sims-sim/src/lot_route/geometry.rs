//! Rectangle operations recovered from the playable 1.91 capture. These are
//! independently verified components, not a verification of the route wrapper.
use crate::{regions::Region, spatial::object_wall_mask};

/// The original Wine InflateRect uses 32-bit wrapping coordinates.
pub fn inflate(Region([left, top, right, bottom, cost]): Region, by: i32) -> Region {
    Region([
        left.wrapping_sub(by),
        top.wrapping_sub(by),
        right.wrapping_add(by),
        bottom.wrapping_add(by),
        cost,
    ])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dynamic {
    pub rectangle: Region,
    pub excluded: bool,
}

/// Original 0x482608..0x4827a5/0x4827b3, including the recovered conditional
/// branches and rectangle imports. Enumeration, callbacks, padding selection
/// and insertion belong to the caller and have separate evidence.
pub fn dynamic(
    rectangle: Region,
    actor_bounds: Region,
    kind: i16,
    wall_mask: i16,
    direction: i16,
    padding: i32,
) -> Dynamic {
    let mut result = rectangle;
    if kind == 8 {
        let mask = object_wall_mask(wall_mask, direction);
        let [left, top, right, bottom, _] = rectangle.0;
        // Each selected edge overwrites the same local rectangle. The order
        // matters when the object's rotated mask contains multiple edges.
        for (bit, edge) in [
            (1, [left, top, right, top.wrapping_add(1), 10000]),
            (4, [right.wrapping_sub(1), top, right, bottom, 10000]),
            (16, [left, bottom.wrapping_sub(1), right, bottom, 10000]),
            (64, [left, top, left.wrapping_add(1), bottom, 10000]),
        ] {
            if mask & bit != 0 {
                result = inflate(Region(edge), padding);
            }
        }
    }
    // Wine IntersectRect rejects empty intersections, including a degenerate
    // input rectangle. Region's graph-overlap predicate also handles seeds
    // and intentionally has a different empty-rectangle contract.
    let a = result.0;
    let b = actor_bounds.0;
    let excluded = a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3]);
    if !excluded && (kind == 2 || result.0[4] >= 8) {
        result = inflate(result, padding);
    }
    Dynamic {
        rectangle: result,
        excluded,
    }
}

/// Original 0x4818af tail: inflate only the hard prefix; soft terrain retains
/// its coordinates and cost. The prefix was established by room preparation.
pub fn finalize(prepared: super::Prepared) -> Result<Vec<Region>, String> {
    if prepared.hard_count > prepared.rectangles.len() {
        return Err("route hard prefix exceeds rectangle count".into());
    }
    Ok(prepared
        .rectangles
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            if i < prepared.hard_count {
                inflate(r, 7)
            } else {
                r
            }
        })
        .collect())
}
