//! The original's `std::sort`, with its tie order.
//!
//! The executable's compiler library sorts with an introsort, and every
//! instantiation has the same shape. This module ports the one at 0x449cbc
//! (pointers, compared through a function); 0x4eb3d1 and 0x42ffce are other
//! instantiations of it. Equal elements do not keep their order once a
//! range is longer than 16, so Rust's sorts cannot stand in where the
//! comparator can tie.
//!
//! - 0x449cbc: the depth limit is twice the floor of log2 of the length,
//!   then the introsort loop (0x44a074), then the final insertion sort
//!   (0x44a120).
//! - 0x44a074: while more than 16 elements are left, it heap-sorts the range
//!   if the depth is spent (0x44a600). Otherwise it takes the median of the
//!   first, middle (`len / 2`) and last elements (0x44a272), partitions
//!   around that value without moving it apart (0x44a307), sorts the right
//!   part by recursion and loops on the left.
//! - 0x44a120: inserts the first 16 elements with a guard (0x44a694), then
//!   the rest without one (0x44a762). The minimum is among the first 16 by
//!   then, so this is a plain insertion sort with the same moves.
//! - 0x44a600: makes a heap (0x44a83f, parents from `(len - 2) / 2` down),
//!   then pops it (0x44a7d9). The sift (0x44a8a4) moves the larger child up,
//!   the left one only when the right is less than it, and then pushes the
//!   held value up (0x44a949).

/// Sorts `items` as the original's `std::sort` does, `less` being its
/// comparator.
pub fn sort_by<T: Copy>(items: &mut [T], less: impl Fn(&T, &T) -> bool) {
    if items.is_empty() {
        return;
    }
    let depth = items.len().ilog2() as usize * 2;
    sort_with_depth(items, depth, &less);
}

fn sort_with_depth<T: Copy>(items: &mut [T], depth: usize, less: &impl Fn(&T, &T) -> bool) {
    introsort(items, depth, less);
    for i in 1..items.len() {
        let value = items[i];
        let mut hole = i;
        while hole > 0 && less(&value, &items[hole - 1]) {
            items[hole] = items[hole - 1];
            hole -= 1;
        }
        items[hole] = value;
    }
}

/// 0x44a272.
fn median<T: Copy>(a: T, b: T, c: T, less: &impl Fn(&T, &T) -> bool) -> T {
    if less(&a, &b) {
        if less(&b, &c) {
            b
        } else if less(&a, &c) {
            c
        } else {
            a
        }
    } else if less(&a, &c) {
        a
    } else if less(&b, &c) {
        c
    } else {
        b
    }
}

/// 0x44a074 with the partition 0x44a307.
fn introsort<T: Copy>(mut items: &mut [T], mut depth: usize, less: &impl Fn(&T, &T) -> bool) {
    while items.len() > 16 {
        if depth == 0 {
            heap_sort(items, less);
            return;
        }
        depth -= 1;
        let last = items.len() - 1;
        let pivot = median(items[0], items[items.len() / 2], items[last], less);
        let mut left = 0;
        let mut right = items.len();
        let cut = loop {
            while less(&items[left], &pivot) {
                left += 1;
            }
            right -= 1;
            while less(&pivot, &items[right]) {
                right -= 1;
            }
            if left >= right {
                break left;
            }
            items.swap(left, right);
            left += 1;
        };
        let (low, high) = items.split_at_mut(cut);
        introsort(high, depth, less);
        items = low;
    }
}

/// 0x44a8a4 then 0x44a949: sifts the hole at `hole` down to a leaf, then
/// pushes `value` up from there, no higher than `hole` was.
fn adjust_heap<T: Copy>(
    items: &mut [T],
    mut hole: usize,
    value: T,
    less: &impl Fn(&T, &T) -> bool,
) {
    let top = hole;
    let mut second = hole * 2 + 2;
    while second < items.len() {
        if less(&items[second], &items[second - 1]) {
            second -= 1;
        }
        items[hole] = items[second];
        hole = second;
        second = second * 2 + 2;
    }
    if second == items.len() {
        items[hole] = items[second - 1];
        hole = second - 1;
    }
    while hole > top {
        let parent = (hole - 1) / 2;
        if !less(&items[parent], &value) {
            break;
        }
        items[hole] = items[parent];
        hole = parent;
    }
    items[hole] = value;
}

/// 0x44a600: make the heap (0x44a83f), then pop it (0x44a7d9).
fn heap_sort<T: Copy>(items: &mut [T], less: &impl Fn(&T, &T) -> bool) {
    let len = items.len();
    if len >= 2 {
        let mut parent = (len - 2) / 2;
        loop {
            adjust_heap(items, parent, items[parent], less);
            if parent == 0 {
                break;
            }
            parent -= 1;
        }
    }
    for end in (2..=len).rev() {
        let value = items[end - 1];
        items[end - 1] = items[0];
        adjust_heap(&mut items[..end - 1], 0, value, less);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pairs of (key, tag) compared on the key alone.
    fn keyed(keys: &[u8]) -> Vec<(u8, usize)> {
        keys.iter().copied().zip(0..).collect()
    }

    fn by_key(a: &(u8, usize), b: &(u8, usize)) -> bool {
        a.0 < b.0
    }

    #[test]
    fn up_to_16_elements_is_an_insertion_sort_and_keeps_ties() {
        let mut items = keyed(&[3, 1, 3, 0, 1, 2, 3, 0, 2, 1, 0, 3, 2, 1, 0, 3]);
        sort_by(&mut items, by_key);
        let tags: Vec<usize> = items.iter().map(|&(_, t)| t).collect();
        assert_eq!(tags, [3, 7, 10, 14, 1, 4, 9, 13, 5, 8, 12, 0, 2, 6, 11, 15]);
    }

    /// 17 elements, all equal: the partition swaps around the middle, so
    /// ties come out in the original's order, not the input's. Worked by
    /// hand through 0x44a307: left and right meet after swapping
    /// 0↔16, 1↔15, ... 7↔9; the cut is at 8 and the insertion pass keeps
    /// it.
    #[test]
    fn longer_ranges_reorder_ties_as_the_partition_does() {
        let mut items = keyed(&[5; 17]);
        sort_by(&mut items, by_key);
        let tags: Vec<usize> = items.iter().map(|&(_, t)| t).collect();
        assert_eq!(
            tags,
            [16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
        );
    }

    #[test]
    fn every_path_sorts() {
        // A small linear congruential sequence, with many ties.
        let mut seed = 12345u32;
        let mut keys = Vec::new();
        for _ in 0..500 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            keys.push((seed >> 16) as u8 % 7);
        }
        for depth in [0, 1, 4, usize::MAX] {
            let mut items = keyed(&keys);
            sort_with_depth(&mut items, depth, &by_key);
            assert!(items.windows(2).all(|w| w[0].0 <= w[1].0), "depth {depth}");
            let mut tags: Vec<usize> = items.iter().map(|&(_, t)| t).collect();
            tags.sort_unstable();
            assert!(tags.iter().copied().eq(0..keys.len()), "depth {depth}");
        }
    }

    /// The heap path (depth spent at once) on 17 equal elements, by hand
    /// through 0x44a83f and 0x44a7d9.
    #[test]
    fn the_heap_path_has_its_own_tie_order() {
        let mut items = keyed(&[1; 17]);
        sort_with_depth(&mut items, 0, &by_key);
        let tags: Vec<usize> = items.iter().map(|&(_, t)| t).collect();
        assert_eq!(tags, items_after_heap_sort(17));
        assert_ne!(tags, (0..17).collect::<Vec<_>>());
    }

    /// A direct model of 0x44a600 on all-equal elements, for the test
    /// above: with no element less than another, the sift always takes the
    /// right child, and the push never moves.
    fn items_after_heap_sort(len: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..len).collect();
        let sift = |v: &mut Vec<usize>, mut hole: usize, len: usize, value: usize| {
            let mut second = hole * 2 + 2;
            while second < len {
                v[hole] = v[second];
                hole = second;
                second = second * 2 + 2;
            }
            if second == len {
                v[hole] = v[second - 1];
                hole = second - 1;
            }
            v[hole] = value;
        };
        let mut parent = (len - 2) / 2;
        loop {
            let value = v[parent];
            sift(&mut v, parent, len, value);
            if parent == 0 {
                break;
            }
            parent -= 1;
        }
        for end in (2..=len).rev() {
            let value = v[end - 1];
            v[end - 1] = v[0];
            sift(&mut v, 0, end - 1, value);
        }
        v
    }
}
