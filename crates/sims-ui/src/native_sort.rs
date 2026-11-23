//! The `std::sort` instantiation `Sims.exe` 1.91 links for the Build Mode
//! catalog strip (`0x501a69`).
//!
//! It is not stable: elements the predicate calls equal end up in an order
//! fixed by its exact steps, and the native strip shows that order. The Floor
//! tool re-sorts its price-and-name ordered list by price alone (`0x5013ea`,
//! key `0` → `0x5c1851` = `+0x1c` price), so equal-price floors appear in the
//! permutation this routine leaves, not alphabetically. Reproducing that
//! needs the same algorithm step for step:
//!
//! - `0x501a69`: depth budget `2 · ⌊log2 n⌋`, then the partition loop, then
//!   the final insertion sort.
//! - `0x501b21`: while more than 16 elements remain, take the median of the
//!   first, middle and last elements (`0x501c12`), Hoare-partition around a
//!   copy of it (`0x501cf4`), recurse on the right part and loop on the left.
//!   An exhausted budget heap-sorts the range (`0x501d3c`).
//! - `0x501ba4`: guarded insertion over the first 16 (`0x501da3`), unguarded
//!   insertion for the rest (`0x501e08`).
//! - Heap: `0x501e55` make, `0x501eec` adjust, `0x501f50` push, `0x501ea0`
//!   sort.

/// Elements at or below which the partition loop stops (`cmp eax, 0x40`).
const ISORT_MAX: usize = 16;

/// Sorts `v` with `less` exactly as `0x501a69` does, including where it
/// leaves elements that compare equal.
pub fn sort_by<T: Copy>(v: &mut [T], less: impl Fn(&T, &T) -> bool) {
    let n = v.len();
    if n == 0 {
        return;
    }
    // `0x501a7e`: count halvings until one element is left.
    let mut ideal = 0usize;
    let mut count = n;
    while count != 1 {
        ideal += 1;
        count >>= 1;
    }
    sort_loop(v, 0, n, 2 * ideal, &less);
    insertion_sort(v, 0, n, &less);
}

/// `0x501b21`.
fn sort_loop<T: Copy>(
    v: &mut [T],
    first: usize,
    mut last: usize,
    mut ideal: usize,
    less: &impl Fn(&T, &T) -> bool,
) {
    while last - first > ISORT_MAX {
        if ideal == 0 {
            heap_sort(v, first, last, less);
            return;
        }
        ideal -= 1;
        let mid = first + (last - first) / 2;
        let pivot = v[median(v, first, mid, last - 1, less)];
        let cut = partition(v, first, last, pivot, less);
        sort_loop(v, cut, last, ideal, less);
        last = cut;
    }
}

/// `0x501c12`: index of the median of three.
fn median<T: Copy>(v: &[T], x: usize, y: usize, z: usize, less: &impl Fn(&T, &T) -> bool) -> usize {
    if less(&v[x], &v[y]) {
        if less(&v[y], &v[z]) {
            y
        } else if less(&v[x], &v[z]) {
            z
        } else {
            x
        }
    } else if less(&v[x], &v[z]) {
        x
    } else if less(&v[y], &v[z]) {
        z
    } else {
        y
    }
}

/// `0x501cf4`: Hoare partition around a copy of the pivot.
fn partition<T: Copy>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    pivot: T,
    less: &impl Fn(&T, &T) -> bool,
) -> usize {
    loop {
        while less(&v[first], &pivot) {
            first += 1;
        }
        loop {
            last -= 1;
            if !less(&pivot, &v[last]) {
                break;
            }
        }
        if first >= last {
            return first;
        }
        v.swap(first, last);
        first += 1;
    }
}

/// `0x501ba4`.
fn insertion_sort<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if last - first > ISORT_MAX {
        for i in first + 1..first + ISORT_MAX {
            guarded_insert(v, first, i, less);
        }
        for i in first + ISORT_MAX..last {
            unguarded_insert(v, i, less);
        }
    } else {
        for i in first + 1..last {
            guarded_insert(v, first, i, less);
        }
    }
}

/// `0x501da3`: inserts `v[i]` into the sorted `v[first..i]`.
fn guarded_insert<T: Copy>(v: &mut [T], first: usize, i: usize, less: &impl Fn(&T, &T) -> bool) {
    let val = v[i];
    if less(&val, &v[first]) {
        v.copy_within(first..i, first + 1);
        v[first] = val;
    } else {
        unguarded_insert(v, i, less);
    }
}

/// `0x501e08` (one step): shifts `v[i]` left past every larger element. A
/// smaller-or-equal element always exists to its left.
fn unguarded_insert<T: Copy>(v: &mut [T], i: usize, less: &impl Fn(&T, &T) -> bool) {
    let val = v[i];
    let mut hole = i;
    while less(&val, &v[hole - 1]) {
        v[hole] = v[hole - 1];
        hole -= 1;
    }
    v[hole] = val;
}

/// `0x501d3c` with `mid == last`: make a heap, then sort it.
fn heap_sort<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    let heap = &mut v[first..last];
    let len = heap.len();
    // `0x501e55`.
    if len >= 2 {
        let mut parent = (len - 2) / 2;
        loop {
            let val = heap[parent];
            adjust_heap(heap, parent, len, val, less);
            if parent == 0 {
                break;
            }
            parent -= 1;
        }
    }
    // `0x501ea0`.
    let mut end = len;
    while end > 1 {
        let val = heap[end - 1];
        heap[end - 1] = heap[0];
        end -= 1;
        adjust_heap(heap, 0, end, val, less);
    }
}

/// `0x501eec`, then `0x501f50`.
fn adjust_heap<T: Copy>(
    heap: &mut [T],
    mut hole: usize,
    len: usize,
    val: T,
    less: &impl Fn(&T, &T) -> bool,
) {
    let top = hole;
    let mut child = 2 * hole + 2;
    while child < len {
        if less(&heap[child], &heap[child - 1]) {
            child -= 1;
        }
        heap[hole] = heap[child];
        hole = child;
        child = 2 * child + 2;
    }
    if child == len {
        heap[hole] = heap[len - 1];
        hole = len - 1;
    }
    // Push the value back up toward `top`.
    while hole > top {
        let parent = (hole - 1) / 2;
        if !less(&heap[parent], &val) {
            break;
        }
        heap[hole] = heap[parent];
        hole = parent;
    }
    heap[hole] = val;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        *seed >> 16
    }

    #[test]
    fn it_sorts_by_the_key() {
        let mut seed = 7;
        for n in [0, 1, 2, 3, 15, 16, 17, 40, 257, 1000] {
            let mut v: Vec<(u32, usize)> = (0..n).map(|i| (lcg(&mut seed) % 9, i)).collect();
            sort_by(&mut v, |a, b| a.0 < b.0);
            assert!(v.windows(2).all(|w| w[0].0 <= w[1].0), "n = {n}");
            let mut ids: Vec<usize> = v.iter().map(|e| e.1).collect();
            ids.sort_unstable();
            assert_eq!(ids, (0..n).collect::<Vec<_>>(), "a permutation, n = {n}");
        }
    }

    /// `(key, index)` pairs: `n` keys from the test LCG, modulo `m`.
    fn keyed(seed: u32, n: usize, m: u32) -> Vec<(u32, usize)> {
        let mut seed = seed;
        (0..n).map(|i| (lcg(&mut seed) % m, i)).collect()
    }

    fn ids(v: &[(u32, usize)]) -> Vec<usize> {
        v.iter().map(|e| e.1).collect()
    }

    // The permutations below are fixed data, produced offline: the 1.91
    // routines (`0x501a69`, and `0x501b21` with an exhausted budget) were
    // run under the unicorn x86 emulator over the shipped image, with the
    // predicate call (`0x501c80`) answered by the same keys. No test here
    // runs the emulator; these compare the port against those results.

    #[test]
    fn the_whole_sort_leaves_the_native_permutation() {
        let mut v = keyed(3, 100, 4);
        sort_by(&mut v, |a, b| a.0 < b.0);
        #[rustfmt::skip]
        let native_100 = [
            19, 66, 21, 22, 23, 24, 64, 26, 62, 61, 29, 57, 56, 54, 51, 49, 39, 42, 3, 17, 16, 15,
            99, 74, 75, 92, 5, 96, 8, 88, 94, 50, 47, 46, 45, 52, 53, 89, 59, 86, 79, 78, 69, 68,
            27, 28, 10, 31, 6, 12, 14, 13, 37, 2, 40, 41, 43, 44, 76, 73, 77, 11, 80, 81, 82, 83,
            84, 90, 91, 4, 93, 98, 33, 35, 32, 60, 63, 25, 65, 67, 72, 1, 97, 95, 38, 48, 36, 0,
            34, 55, 7, 87, 9, 85, 58, 30, 20, 18, 70, 71,
        ];
        assert_eq!(ids(&v), native_100);
        let mut v = keyed(5, 257, 7);
        sort_by(&mut v, |a, b| a.0 < b.0);
        #[rustfmt::skip]
        let native_257 = [
            120, 117, 113, 129, 112, 110, 106, 85, 149, 81, 76, 157, 75, 73, 181, 16, 0, 58, 250,
            245, 240, 233, 230, 227, 215, 202, 196, 188, 80, 82, 72, 100, 67, 108, 59, 55, 256,
            254, 246, 241, 221, 171, 166, 155, 154, 153, 148, 144, 141, 140, 136, 133, 132, 128,
            127, 34, 49, 47, 18, 21, 41, 29, 7, 35, 46, 124, 122, 119, 201, 115, 205, 38, 96, 93,
            92, 126, 190, 137, 138, 139, 28, 142, 26, 183, 182, 179, 177, 162, 167, 19, 8, 237,
            238, 239, 6, 68, 5, 4, 62, 1, 88, 247, 248, 236, 255, 193, 160, 189, 15, 31, 30, 187,
            135, 150, 159, 20, 243, 244, 186, 65, 185, 251, 146, 176, 220, 11, 99, 42, 217, 216,
            12, 86, 94, 211, 109, 39, 210, 198, 91, 121, 79, 9, 229, 83, 203, 89, 173, 169, 222, 2,
            175, 226, 252, 224, 178, 180, 218, 184, 213, 212, 10, 228, 13, 200, 231, 194, 143, 32,
            125, 36, 116, 114, 40, 107, 101, 43, 84, 78, 74, 71, 70, 66, 52, 161, 23, 152, 165,
            156, 206, 242, 158, 87, 163, 22, 50, 214, 170, 48, 235, 174, 225, 98, 97, 95, 223, 90,
            232, 25, 57, 134, 253, 130, 45, 77, 249, 60, 61, 63, 172, 123, 199, 118, 164, 204, 51,
            234, 168, 69, 64, 3, 53, 56, 54, 147, 145, 27, 131, 17, 191, 192, 33, 24, 195, 14, 197,
            151, 37, 207, 208, 209, 111, 105, 104, 103, 102, 219, 44,
        ];
        assert_eq!(ids(&v), native_257);
    }

    #[test]
    fn an_exhausted_budget_heap_sorts_as_native_does() {
        // `0x501b21` called with a depth budget of 0 on 40 elements goes
        // straight to the heap sort (`0x501d3c`), with no insertion pass.
        let mut v = keyed(11, 40, 3);
        sort_loop(&mut v, 0, 40, 0, &|a: &(u32, usize), b: &(u32, usize)| {
            a.0 < b.0
        });
        #[rustfmt::skip]
        let native = [
            27, 34, 1, 29, 8, 17, 6, 32, 36, 31, 9, 15, 10, 22, 24, 39, 33, 21, 25, 5, 23, 11, 35,
            2, 30, 7, 16, 37, 18, 38, 4, 19, 20, 3, 12, 26, 0, 13, 28, 14,
        ];
        assert_eq!(ids(&v), native);
    }

    #[test]
    fn a_short_run_of_equal_keys_keeps_its_order() {
        // Sixteen or fewer elements only see the insertion sort, which never
        // moves an element past an equal one.
        let mut v: Vec<(u32, usize)> = (0..16).map(|i| (u32::from(i >= 8), i)).collect();
        v.reverse();
        sort_by(&mut v, |a, b| a.0 < b.0);
        let ids: Vec<usize> = v.iter().map(|e| e.1).collect();
        assert_eq!(ids, [7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8]);
    }

    #[test]
    fn a_long_run_of_equal_keys_is_permuted_by_the_partition() {
        // 18 equal keys after one smaller. The median of v[0], v[9], v[18]
        // is v[18], so the pivot has the run's key; the Hoare partition then
        // swaps v[1]↔v[18], v[2]↔v[17], … and the insertion sort keeps
        // that, so the run comes back reversed.
        let mut v: Vec<(u32, usize)> = (0..19).map(|i| (u32::from(i > 0), i)).collect();
        sort_by(&mut v, |a, b| a.0 < b.0);
        let ids: Vec<usize> = v.iter().map(|e| e.1).collect();
        let reversed: Vec<usize> = std::iter::once(0).chain((1..19).rev()).collect();
        assert_eq!(ids, reversed);
    }
}
