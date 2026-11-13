//! Win32 inputs are supplied by the host. The observed Wine rectangle predicate
//! is ordinary signed comparison; it never subtracts potentially extreme edges.
pub fn is_rect_empty(rectangle: Option<[i32; 4]>) -> u32 {
    u32::from(rectangle.is_none_or(|[left, top, right, bottom]| left >= right || top >= bottom))
}
