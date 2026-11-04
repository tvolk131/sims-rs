//!4c8049: select descriptor pairs inside the retained shadow-kernel radius.
//! Preserve untouched output words, including the native second-only oddity.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub offset: usize,
    pub width: u8,
    pub value: u32,
}
pub fn classify(d: [i8; 4], radius: i32, mut outputs: [i32; 4]) -> (u32, [i32; 4], Vec<Write>) {
    let mut count = u32::from(d[0] != 127) + u32::from(d[2] != 127);
    let mut writes = vec![];
    if count == 0 {
        return (count, outputs, writes);
    }
    let mut put = |i: usize, v: i32| {
        outputs[i] = v;
        writes.push(Write {
            offset: i * 4,
            width: 4,
            value: v as u32,
        });
    };
    put(0, i32::from(d[0]));
    put(1, i32::from(d[1]));
    let inside = |x: i8, y: i8| i32::from(x).abs() <= radius && i32::from(y).abs() <= radius;
    if count == 2 {
        put(2, i32::from(d[2]));
        put(3, i32::from(d[3]));
        if !inside(d[2], d[3]) {
            count = 1;
        }
        if !inside(d[0], d[1]) {
            if count == 2 {
                put(0, i32::from(d[2]));
                put(1, i32::from(d[3]));
                count = 1;
            } else {
                count = 0;
            }
        }
    } else if !inside(d[0], d[1]) {
        count = 0;
    }
    (count, outputs, writes)
}
