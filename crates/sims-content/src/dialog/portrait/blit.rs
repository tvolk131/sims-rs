//! 5dee3e's RGB-image clipping before the DirectDraw surface endpoint.
use crate::dialog::portrait::Rect;

pub fn rectangles(
    size: [i32; 2],
    mut from: Rect,
    mut to: Rect,
    clip: Option<Rect>,
) -> Option<(Rect, Rect)> {
    let over = to[2].wrapping_sub(size[0]);
    if over > 0 {
        from[2] = from[2].wrapping_sub(over);
        to[2] = to[2].wrapping_sub(over);
        if to[2] <= to[0] {
            return None;
        }
    }
    let over = to[0].wrapping_neg();
    if over > 0 {
        from[0] = from[0].wrapping_add(over);
        to[0] = 0;
        if to[2] < 1 {
            return None;
        }
    }
    let over = to[3].wrapping_sub(size[1]);
    if over > 0 {
        from[3] = from[3].wrapping_sub(over);
        to[3] = to[3].wrapping_sub(over);
        if to[3] <= to[1] {
            return None;
        }
    }
    let over = to[1].wrapping_neg();
    if over > 0 {
        from[1] = from[1].wrapping_add(over);
        to[1] = 0;
        if to[3] < 1 {
            return None;
        }
    }
    if let Some(clip) = clip {
        for (edge, sign) in [(2, 1), (0, -1), (3, 1), (1, -1)] {
            let over = if sign == 1 {
                to[edge].wrapping_sub(clip[edge])
            } else {
                clip[edge].wrapping_sub(to[edge])
            };
            if over > 0 {
                let change = over.wrapping_mul(-sign);
                to[edge] = to[edge].wrapping_add(change);
                from[edge] = from[edge].wrapping_add(change);
            }
        }
    }
    (to[0] < to[2] && to[1] < to[3]).then_some((from, to))
}
