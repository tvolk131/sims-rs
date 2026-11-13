//! Native nearby-lot ordering, 0x428350 and 0x42ffce. The coordinate vector is
//! an explicit load-context input; this module does not invent missing entries.

/// Exclude lot zero and the loaded lot, then preserve the original sort's tie
/// ordering. Coordinates are indexed by lot number, including unused entries.
pub fn order(coordinates: &[[i32; 2]], house: i32) -> Result<Vec<i32>, String> {
    if coordinates.len() > 32768 {
        return Err("nearby-house coordinates exceed supported lot range".into());
    }
    if house == 0 {
        return Ok(vec![]);
    }
    let mut lots: Vec<_> = (1..coordinates.len() as i32)
        .filter(|&id| id != house)
        .collect();
    let less = comparator(coordinates, house);
    sort_by(&mut lots, &less);
    Ok(lots)
}

/// The same compiler sort occurs at 449cbc for presenter pointers. Preserve
/// partition and heap tie choices; Rust's unstable sort is not interchangeable.
pub(crate) fn sort_by<T: Copy>(lots: &mut [T], less: &impl Fn(T, T) -> bool) {
    if !lots.is_empty() {
        let depth = (lots.len().ilog2() * 2) as usize;
        sort_with_depth(lots, depth, less);
    }
}

pub(crate) fn sort_with_depth<T: Copy>(lots: &mut [T], depth: usize, less: &impl Fn(T, T) -> bool) {
    introsort(lots, depth, less);
    // 0x4306b0 uses guarded insertion for the first 16 elements and then
    // unguarded insertion. The established minimum makes these equivalent
    // to this bounded insertion pass, including equal-element order.
    for i in 1..lots.len() {
        let value = lots[i];
        let mut dest = i;
        while dest > 0 && less(value, lots[dest - 1]) {
            lots[dest] = lots[dest - 1];
            dest -= 1;
        }
        lots[dest] = value;
    }
}

fn comparator(coordinates: &[[i32; 2]], house: i32) -> impl Fn(i32, i32) -> bool + '_ {
    // 0x42886e returns (0,0) for zero, negative or out-of-range IDs.
    let point = |id: i32| {
        if id > 0 {
            coordinates.get(id as usize).copied().unwrap_or([0, 0])
        } else {
            [0, 0]
        }
    };
    let [x, y] = point(house);
    move |a, b| {
        let distance = |id| {
            let [px, py] = point(id);
            let dx = px.wrapping_sub(x);
            let dy = py.wrapping_sub(y);
            // x86 IMUL and ADD narrow before the signed SETL comparison.
            dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
        };
        distance(a) < distance(b)
    }
}

fn median<T: Copy>(a: T, b: T, c: T, less: &impl Fn(T, T) -> bool) -> T {
    // 0x54186d: tie choices matter to the subsequent partition swaps.
    if less(a, b) {
        if less(b, c) {
            b
        } else if less(a, c) {
            c
        } else {
            a
        }
    } else if less(a, c) {
        a
    } else if less(b, c) {
        c
    } else {
        b
    }
}

fn introsort<T: Copy>(mut lots: &mut [T], mut depth: usize, less: &impl Fn(T, T) -> bool) {
    while lots.len() > 16 {
        if depth == 0 {
            heap_sort(lots, less);
            return;
        }
        depth -= 1;
        let pivot = median(lots[0], lots[lots.len() / 2], lots[lots.len() - 1], less);
        // 0x430a95 partitions without moving a distinguished pivot element.
        let mut left = 0;
        let mut right = lots.len();
        let cut = loop {
            while less(lots[left], pivot) {
                left += 1;
            }
            right -= 1;
            while less(pivot, lots[right]) {
                right -= 1;
            }
            if left >= right {
                break left;
            }
            lots.swap(left, right);
            left += 1;
        };
        let (low, high) = lots.split_at_mut(cut);
        introsort(high, depth, less);
        lots = low;
    }
}

fn adjust_heap<T: Copy>(lots: &mut [T], mut hole: usize, value: T, less: &impl Fn(T, T) -> bool) {
    // 0x430f0a moves the larger child up, preferring the right on equality,
    // then 0x430fb7 inserts the held value up from the final hole.
    let top = hole;
    let mut right = hole * 2 + 2;
    while right < lots.len() {
        let child = if less(lots[right], lots[right - 1]) {
            right - 1
        } else {
            right
        };
        lots[hole] = lots[child];
        hole = child;
        right = hole * 2 + 2;
    }
    if right == lots.len() {
        lots[hole] = lots[right - 1];
        hole = right - 1;
    }
    while hole > top {
        let parent = (hole - 1) / 2;
        if !less(lots[parent], value) {
            break;
        }
        lots[hole] = lots[parent];
        hole = parent;
    }
    lots[hole] = value;
}

fn heap_sort<T: Copy>(lots: &mut [T], less: &impl Fn(T, T) -> bool) {
    if lots.len() < 2 {
        return;
    }
    for parent in (0..=(lots.len() - 2) / 2).rev() {
        adjust_heap(lots, parent, lots[parent], less);
    }
    for end in (1..lots.len()).rev() {
        let value = lots[end];
        lots[end] = lots[0];
        adjust_heap(&mut lots[..end], 0, value, less);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    struct Case {
        coordinates: Vec<[i32; 2]>,
        house: i32,
        #[serde(default)]
        heap: bool,
    }
    #[derive(Deserialize)]
    struct Original {
        order: Vec<i32>,
        cleared: bool,
    }
    #[derive(Deserialize)]
    struct Row {
        input: Case,
        original: Original,
    }
    #[derive(Deserialize)]
    struct Report {
        capture_sha256: String,
        cases: Vec<Row>,
    }

    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_order_and_distance_ties() {
        let report: Report =
            serde_json::from_str(include_str!("../tests/fixtures/nearby-house-order.json"))
                .unwrap();
        assert_eq!(
            report.capture_sha256,
            "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
        );
        assert_eq!(report.cases.len(), 230);
        for row in report.cases {
            assert_eq!(
                row.original.cleared,
                row.input.house == 0 && !row.input.heap
            );
            let actual = if row.input.heap {
                let mut lots: Vec<_> = (1..row.input.coordinates.len() as i32)
                    .filter(|&id| id != row.input.house)
                    .collect();
                heap_sort(
                    &mut lots,
                    &comparator(&row.input.coordinates, row.input.house),
                );
                lots
            } else {
                order(&row.input.coordinates, row.input.house).unwrap()
            };
            assert_eq!(actual, row.original.order, "{:?}", row.input);
        }
    }
}
