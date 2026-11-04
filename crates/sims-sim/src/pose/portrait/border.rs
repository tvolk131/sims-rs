//! 4c357c / 4c362e / 4c3738: exact clear-color margins. Original surfaces
//! supply u16 pixels; other backends can use the same equality-only traversal.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scan {
    pub margins: [i32; 4],
    pub success: bool,
}

/// `None` represents a failed native surface lock: retain zero margins and
/// still evaluate the rectangle test. Stride is in pixels, not bytes.
pub fn scan<T: PartialEq>(
    pixels: Option<&[T]>,
    stride: usize,
    rect: [usize; 4],
    background: &T,
) -> Result<Scan, String> {
    let [x, y, right, bottom] = rect;
    if x > right || y > bottom || rect.iter().any(|&n| n > i32::MAX as usize) {
        return Err("invalid portrait border rectangle".into());
    }
    let width = right - x;
    let height = bottom - y;
    let mut margins = [0; 4];
    if let Some(pixels) = pixels {
        if right > stride
            || (height > 0
                && (bottom - 1)
                    .checked_mul(stride)
                    .and_then(|n| n.checked_add(right))
                    .is_none_or(|end| end > pixels.len()))
        {
            return Err("portrait border pixels outside surface".into());
        }
        let row = |n: usize| &pixels[n * stride + x..n * stride + right];
        let mut top = 0;
        while top < height && row(y + top).iter().all(|p| p == background) {
            top += 1;
        }
        let mut lower = 0;
        if top != height {
            while lower < height - top && row(bottom - 1 - lower).iter().all(|p| p == background) {
                lower += 1;
            }
        }
        let mut left = width;
        let mut trailing = width;
        for n in y + top..bottom - lower {
            let row = row(n);
            left = left.min(row.iter().take_while(|p| *p == background).count());
            // The native right scan stops at the smallest left margin seen so
            // far. Preserve that bound, including fully blank interior rows.
            let mut end = width;
            while end > left && row[end - 1] == *background {
                end -= 1;
            }
            trailing = trailing.min(width - end);
        }
        margins = [left as i32, top as i32, trailing as i32, lower as i32];
    }
    Ok(Scan {
        success: i64::from(margins[0]) + i64::from(margins[2]) < width as i64
            && i64::from(margins[1]) + i64::from(margins[3]) < height as i64,
        margins,
    })
}
