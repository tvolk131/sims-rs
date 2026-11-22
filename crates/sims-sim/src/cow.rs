//! Copy-on-write containers backed by [`Arc`] with wire-compatible `serde`
//! serialization.
//!
//! `World::clone` runs before every interactive tick (`Lot::step_in` when
//! `asks = true`) and before speculative player commands (`buy_move`,
//! `pie_menu`, `grab`, `drop`, etc.). Wrapping the world's large, mostly-static
//! maps and tables in [`CowMap`] and [`CowBox`] makes cloning `O(1)` while
//! preserving bit-for-bit identical `postcard` serialization and snapshot
//! hashes.
//!
//! [`CowMap`] stores low `u16` keys (`0..4096`) in 64-entry copy-on-write pages
//! (`Arc<Page<V>>`) with a `u64` occupancy bitmask and rare high keys (`>= 4096`)
//! in an overflow `BTreeMap`. Reads are `O(1)` array loads, single-entry writes
//! under a shared clone detach only one 64-entry page instead of the entire
//! map, and iteration/serialization visits keys in exact ascending order.

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::collections::btree_map::{self, Entry};
use std::fmt;
use std::ops::{Deref, DerefMut, Index, RangeInclusive};
use std::sync::Arc;

const PAGE_SHIFT: usize = 6;
const PAGE_SIZE: usize = 1 << PAGE_SHIFT;
const DENSE_PAGES: usize = 64;
const DENSE_LIMIT: usize = PAGE_SIZE * DENSE_PAGES;

const fn make_u16_keys() -> [u16; DENSE_LIMIT] {
    let mut keys = [0u16; DENSE_LIMIT];
    let mut i = 0;
    while i < DENSE_LIMIT {
        keys[i] = i as u16;
        i += 1;
    }
    keys
}

static U16_KEYS: [u16; DENSE_LIMIT] = make_u16_keys();

/// Key type supported by paged [`CowMap`].
pub trait CowKey: Ord + Copy + 'static {
    /// Returns the dense slot index in `0..4096` if this key lives in paged
    /// storage, or `None` if it belongs in the overflow `BTreeMap`.
    fn dense_index(self) -> Option<usize>;

    /// Returns a static reference to the key for dense slot `index`.
    fn key_ref(index: usize) -> &'static Self;
}

impl CowKey for u16 {
    #[inline]
    fn dense_index(self) -> Option<usize> {
        let i = usize::from(self);
        (i < DENSE_LIMIT).then_some(i)
    }

    #[inline]
    fn key_ref(index: usize) -> &'static Self {
        &U16_KEYS[index]
    }
}

/// A copy-on-write single value backed by [`Arc`].
///
/// Serializes and deserializes identically to `T`.
pub struct CowBox<T>(Arc<T>);

impl<T> CowBox<T> {
    #[inline]
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// Returns `true` if `self` and `other` share the same backing allocation.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Short-circuits on `Arc::ptr_eq` before calling `eq`.
    ///
    /// Callers must only use this when `eq` is reflexive for all values
    /// (for example, bit-exact float comparisons via `to_bits()`).
    #[inline]
    pub fn ptr_eq_or(&self, other: &Self, eq: impl FnOnce(&T, &T) -> bool) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || eq(&self.0, &other.0)
    }
}

impl<T> Clone for CowBox<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Default> Default for CowBox<T> {
    #[inline]
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Deref for CowBox<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Clone> DerefMut for CowBox<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

impl<T> AsRef<T> for CowBox<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> AsMut<T> for CowBox<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<T> From<T> for CowBox<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: PartialEq> PartialEq for CowBox<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}

impl<T: Eq> Eq for CowBox<T> {}

impl<T: fmt::Debug> fmt::Debug for CowBox<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: Serialize> Serialize for CowBox<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for CowBox<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::new)
    }
}

struct Page<V> {
    present: u64,
    slots: [Option<Arc<V>>; PAGE_SIZE],
}

impl<V> Page<V> {
    #[inline]
    fn new() -> Self {
        Self {
            present: 0,
            slots: std::array::from_fn(|_| None),
        }
    }
}

impl<V> Clone for Page<V> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            present: self.present,
            slots: self.slots.clone(),
        }
    }
}

struct CowMapInner<K, V> {
    pages: Vec<Option<Arc<Page<V>>>>,
    overflow: BTreeMap<K, Arc<V>>,
    len: usize,
}

impl<K, V> CowMapInner<K, V> {
    #[inline]
    fn new() -> Self {
        Self {
            pages: Vec::new(),
            overflow: BTreeMap::new(),
            len: 0,
        }
    }

    #[inline]
    fn trim_pages(&mut self) {
        while self.pages.last().is_some_and(Option::is_none) {
            self.pages.pop();
        }
    }
}

impl<K: Clone, V> Clone for CowMapInner<K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            pages: self.pages.clone(),
            overflow: self.overflow.clone(),
            len: self.len,
        }
    }
}

/// A paged copy-on-write ordered map backed by [`Arc`].
///
/// Cloning is `O(1)`. Low keys (`0..4096`) live in 64-entry copy-on-write
/// pages (`Arc<Page<V>>`) for `O(1)` reads and page-granular CoW writes;
/// keys `>= 4096` spill into an overflow `BTreeMap`. Serializes and
/// deserializes identically to `BTreeMap<K, V>`.
pub struct CowMap<K, V>(Arc<CowMapInner<K, V>>);

impl<K, V> CowMap<K, V> {
    #[inline]
    pub fn new() -> Self {
        Self(Arc::new(CowMapInner::new()))
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }

    /// Returns `true` if `self` and `other` share the exact same root allocation.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<K: CowKey, V> CowMap<K, V> {
    #[inline]
    pub fn keys(&self) -> CowMapKeys<'_, K, V> {
        CowMapKeys(self.iter())
    }

    #[inline]
    pub fn iter(&self) -> CowMapIter<'_, K, V> {
        CowMapIter {
            pages: &self.0.pages,
            front: 0,
            back_end: self.0.pages.len() << PAGE_SHIFT,
            overflow: self.0.overflow.iter(),
            remaining: self.0.len,
        }
    }

    #[inline]
    pub fn values(&self) -> CowMapValues<'_, K, V> {
        CowMapValues(self.iter())
    }

    #[inline]
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        for (p, page) in self.0.pages.iter().enumerate() {
            if let Some(page) = page
                && page.present != 0
            {
                let bit = page.present.trailing_zeros() as usize;
                let idx = (p << PAGE_SHIFT) | bit;
                let v = page.slots[bit].as_deref()?;
                return Some((K::key_ref(idx), v));
            }
        }
        self.0
            .overflow
            .first_key_value()
            .map(|(k, v)| (k, v.as_ref()))
    }

    #[inline]
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        if let Some((k, v)) = self.0.overflow.last_key_value() {
            return Some((k, v.as_ref()));
        }
        for (p, page) in self.0.pages.iter().enumerate().rev() {
            if let Some(page) = page
                && page.present != 0
            {
                let bit = 63 - page.present.leading_zeros() as usize;
                let idx = (p << PAGE_SHIFT) | bit;
                let v = page.slots[bit].as_deref()?;
                return Some((K::key_ref(idx), v));
            }
        }
        None
    }

    #[inline]
    pub fn contains_key(&self, key: &K) -> bool {
        if let Some(idx) = key.dense_index() {
            let p = idx >> PAGE_SHIFT;
            let s = idx & (PAGE_SIZE - 1);
            self.0
                .pages
                .get(p)
                .and_then(Option::as_ref)
                .is_some_and(|page| (page.present & (1u64 << s)) != 0)
        } else {
            self.0.overflow.contains_key(key)
        }
    }

    #[inline]
    pub fn get(&self, key: &K) -> Option<&V> {
        if let Some(idx) = key.dense_index() {
            let p = idx >> PAGE_SHIFT;
            let s = idx & (PAGE_SIZE - 1);
            self.0.pages.get(p)?.as_ref()?.slots[s].as_deref()
        } else {
            self.0.overflow.get(key).map(Arc::as_ref)
        }
    }

    #[inline]
    pub fn get_key_value(&self, key: &K) -> Option<(&K, &V)> {
        if let Some(idx) = key.dense_index() {
            let p = idx >> PAGE_SHIFT;
            let s = idx & (PAGE_SIZE - 1);
            let v = self.0.pages.get(p)?.as_ref()?.slots[s].as_deref()?;
            Some((K::key_ref(idx), v))
        } else {
            self.0
                .overflow
                .get_key_value(key)
                .map(|(k, v)| (k, v.as_ref()))
        }
    }

    /// Structural comparison that short-circuits on `Arc::ptr_eq` at the map,
    /// page, and entry levels before invoking `eq`.
    ///
    /// Callers must only use this when `eq` is reflexive for all values
    /// (for example, bit-exact float comparisons via `to_bits()`).
    pub fn ptr_eq_or(&self, other: &Self, mut eq: impl FnMut(&V, &V) -> bool) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        if self.0.len != other.0.len {
            return false;
        }
        let max_pages = self.0.pages.len().max(other.0.pages.len());
        for p in 0..max_pages {
            match (
                self.0.pages.get(p).and_then(Option::as_ref),
                other.0.pages.get(p).and_then(Option::as_ref),
            ) {
                (None, None) => {}
                (Some(pa), Some(pb)) => {
                    if Arc::ptr_eq(pa, pb) {
                        continue;
                    }
                    if pa.present != pb.present {
                        return false;
                    }
                    let mut mask = pa.present;
                    while mask != 0 {
                        let s = mask.trailing_zeros() as usize;
                        mask &= mask - 1;
                        let va = pa.slots[s].as_ref().expect("present bit matches slot");
                        let vb = pb.slots[s].as_ref().expect("present bit matches slot");
                        if !Arc::ptr_eq(va, vb) && !eq(va, vb) {
                            return false;
                        }
                    }
                }
                (Some(page), None) | (None, Some(page)) => {
                    if page.present != 0 {
                        return false;
                    }
                }
            }
        }
        if self.0.overflow.len() != other.0.overflow.len() {
            return false;
        }
        self.0
            .overflow
            .iter()
            .zip(other.0.overflow.iter())
            .all(|((ka, va), (kb, vb))| ka == kb && (Arc::ptr_eq(va, vb) || eq(va, vb)))
    }
}

impl<V> CowMap<u16, V> {
    /// Returns the smallest key `>= min_key` present in the map, skipping
    /// empty gaps via per-page occupancy bitmasks.
    #[inline]
    pub fn next_key_from(&self, min_key: u16) -> Option<u16> {
        let start = usize::from(min_key);
        if start < DENSE_LIMIT {
            let start_page = start >> PAGE_SHIFT;
            let start_slot = start & (PAGE_SIZE - 1);
            let mut p = start_page;
            while p < self.0.pages.len() {
                if let Some(page) = &self.0.pages[p] {
                    let mask = if p == start_page {
                        page.present & (!0u64 << start_slot)
                    } else {
                        page.present
                    };
                    if mask != 0 {
                        let bit = mask.trailing_zeros() as usize;
                        return Some(((p << PAGE_SHIFT) | bit) as u16);
                    }
                }
                p += 1;
            }
            self.0.overflow.first_key_value().map(|(&k, _)| k)
        } else {
            self.0.overflow.range(min_key..).next().map(|(&k, _)| k)
        }
    }

    /// Iterates over `(&u16, &V)` entries whose keys fall within `bounds`
    /// in ascending key order.
    #[inline]
    pub fn range(&self, bounds: RangeInclusive<u16>) -> CowMapRange<'_, V> {
        let (start, end) = (*bounds.start(), *bounds.end());
        if start > end {
            return CowMapRange {
                pages: &[],
                front: 0,
                back_end: 0,
                overflow: None,
            };
        }
        let max_dense = self.0.pages.len() << PAGE_SHIFT;
        let front = usize::from(start).min(max_dense);
        let back_end = (usize::from(end) + 1).min(max_dense);
        let overflow = if usize::from(end) >= DENSE_LIMIT {
            let ov_start = start.max(DENSE_LIMIT as u16);
            Some(self.0.overflow.range(ov_start..=end))
        } else {
            None
        };
        CowMapRange {
            pages: &self.0.pages,
            front,
            back_end,
            overflow,
        }
    }
}

/// Borrowing range iterator over `(&u16, &V)` entries in a [`CowMap`].
pub struct CowMapRange<'a, V> {
    pages: &'a [Option<Arc<Page<V>>>],
    front: usize,
    back_end: usize,
    overflow: Option<btree_map::Range<'a, u16, Arc<V>>>,
}

impl<'a, V> Iterator for CowMapRange<'a, V> {
    type Item = (&'a u16, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.front < self.back_end {
            let p = self.front >> PAGE_SHIFT;
            let s = self.front & (PAGE_SIZE - 1);
            let page_end = (p + 1) << PAGE_SHIFT;
            let Some(page) = &self.pages[p] else {
                self.front = page_end.min(self.back_end);
                continue;
            };
            let mut mask = page.present & (!0u64 << s);
            if page_end > self.back_end {
                let end_slot = self.back_end & (PAGE_SIZE - 1);
                mask &= (1u64 << end_slot) - 1;
            }
            if mask == 0 {
                self.front = page_end.min(self.back_end);
                continue;
            }
            let bit = mask.trailing_zeros() as usize;
            let idx = (p << PAGE_SHIFT) | bit;
            self.front = idx + 1;
            let value = page.slots[bit]
                .as_deref()
                .expect("present bit matches slot");
            return Some((&U16_KEYS[idx], value));
        }
        self.overflow.as_mut()?.next().map(|(k, v)| (k, v.as_ref()))
    }
}

/// Borrowing iterator over `(&K, &V)` entries in a [`CowMap`].
pub struct CowMapIter<'a, K, V> {
    pages: &'a [Option<Arc<Page<V>>>],
    front: usize,
    back_end: usize,
    overflow: btree_map::Iter<'a, K, Arc<V>>,
    remaining: usize,
}

impl<'a, K: CowKey, V> Iterator for CowMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.front < self.back_end {
            let p = self.front >> PAGE_SHIFT;
            let s = self.front & (PAGE_SIZE - 1);
            let page_end = (p + 1) << PAGE_SHIFT;
            let Some(page) = &self.pages[p] else {
                self.front = page_end.min(self.back_end);
                continue;
            };
            let mut mask = page.present & (!0u64 << s);
            if page_end > self.back_end {
                let end_slot = self.back_end & (PAGE_SIZE - 1);
                mask &= (1u64 << end_slot) - 1;
            }
            if mask == 0 {
                self.front = page_end.min(self.back_end);
                continue;
            }
            let bit = mask.trailing_zeros() as usize;
            let idx = (p << PAGE_SHIFT) | bit;
            self.front = idx + 1;
            self.remaining -= 1;
            let value = page.slots[bit]
                .as_deref()
                .expect("present bit matches slot");
            return Some((K::key_ref(idx), value));
        }
        let (k, v) = self.overflow.next()?;
        self.remaining -= 1;
        Some((k, v.as_ref()))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K: CowKey, V> DoubleEndedIterator for CowMapIter<'_, K, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if let Some((k, v)) = self.overflow.next_back() {
            self.remaining -= 1;
            return Some((k, v.as_ref()));
        }
        while self.front < self.back_end {
            let last = self.back_end - 1;
            let p = last >> PAGE_SHIFT;
            let s = last & (PAGE_SIZE - 1);
            let page_start = p << PAGE_SHIFT;
            let Some(page) = &self.pages[p] else {
                self.back_end = page_start.max(self.front);
                continue;
            };
            let mut mask = page.present & (!0u64 >> (63 - s));
            if page_start < self.front {
                let start_slot = self.front & (PAGE_SIZE - 1);
                mask &= !0u64 << start_slot;
            }
            if mask == 0 {
                self.back_end = page_start.max(self.front);
                continue;
            }
            let bit = 63 - mask.leading_zeros() as usize;
            let idx = page_start | bit;
            self.back_end = idx;
            self.remaining -= 1;
            let value = page.slots[bit]
                .as_deref()
                .expect("present bit matches slot");
            return Some((K::key_ref(idx), value));
        }
        None
    }
}

impl<K: CowKey, V> ExactSizeIterator for CowMapIter<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.remaining
    }
}

impl<K, V> Clone for CowMapIter<'_, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            pages: self.pages,
            front: self.front,
            back_end: self.back_end,
            overflow: self.overflow.clone(),
            remaining: self.remaining,
        }
    }
}

/// Borrowing iterator over `&K` keys in a [`CowMap`].
pub struct CowMapKeys<'a, K, V>(CowMapIter<'a, K, V>);

impl<'a, K: CowKey, V> Iterator for CowMapKeys<'a, K, V> {
    type Item = &'a K;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, _)| k)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K: CowKey, V> DoubleEndedIterator for CowMapKeys<'_, K, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, _)| k)
    }
}

impl<K: CowKey, V> ExactSizeIterator for CowMapKeys<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<K, V> Clone for CowMapKeys<'_, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Borrowing iterator over `&V` values in a [`CowMap`].
pub struct CowMapValues<'a, K, V>(CowMapIter<'a, K, V>);

impl<'a, K: CowKey, V> Iterator for CowMapValues<'a, K, V> {
    type Item = &'a V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K: CowKey, V> DoubleEndedIterator for CowMapValues<'_, K, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(_, v)| v)
    }
}

impl<K: CowKey, V> ExactSizeIterator for CowMapValues<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<K, V> Clone for CowMapValues<'_, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

type PageSlotsMut<'a, V> = (usize, usize, std::slice::IterMut<'a, Option<Arc<V>>>);

/// Mutable iterator over `(&K, &mut V)` entries in a [`CowMap`].
pub struct CowMapIterMut<'a, K, V> {
    pages: std::slice::IterMut<'a, Option<Arc<Page<V>>>>,
    front_page_idx: usize,
    back_page_idx: usize,
    front_slots: Option<PageSlotsMut<'a, V>>,
    back_slots: Option<PageSlotsMut<'a, V>>,
    overflow: btree_map::IterMut<'a, K, Arc<V>>,
    remaining: usize,
}

impl<'a, K: CowKey, V: Clone> Iterator for CowMapIterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((base, ref mut offset, ref mut slots)) = self.front_slots {
                for slot in slots.by_ref() {
                    let idx = base + *offset;
                    *offset += 1;
                    if let Some(arc) = slot {
                        self.remaining -= 1;
                        return Some((K::key_ref(idx), Arc::make_mut(arc)));
                    }
                }
                self.front_slots = None;
            }
            if let Some(page_opt) = self.pages.next() {
                let p = self.front_page_idx;
                self.front_page_idx += 1;
                if let Some(page_arc) = page_opt
                    && page_arc.present != 0
                {
                    let page = Arc::make_mut(page_arc);
                    self.front_slots = Some((p << PAGE_SHIFT, 0, page.slots.iter_mut()));
                }
                continue;
            }
            if let Some((base, ref mut end, ref mut slots)) = self.back_slots {
                // Consume remaining items from back_slots from the front.
                let start_slot = *end - slots.len();
                for (i, slot) in slots.by_ref().enumerate() {
                    if let Some(arc) = slot {
                        self.remaining -= 1;
                        return Some((K::key_ref(base + start_slot + i), Arc::make_mut(arc)));
                    }
                }
                self.back_slots = None;
            }
            let (k, v) = self.overflow.next()?;
            self.remaining -= 1;
            return Some((k, Arc::make_mut(v)));
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K: CowKey, V: Clone> DoubleEndedIterator for CowMapIterMut<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if let Some((k, v)) = self.overflow.next_back() {
            self.remaining -= 1;
            return Some((k, Arc::make_mut(v)));
        }
        loop {
            if let Some((base, ref mut end, ref mut slots)) = self.back_slots {
                while let Some(slot) = slots.next_back() {
                    *end -= 1;
                    if let Some(arc) = slot {
                        self.remaining -= 1;
                        return Some((K::key_ref(base + *end), Arc::make_mut(arc)));
                    }
                }
                self.back_slots = None;
            }
            if let Some(page_opt) = self.pages.next_back() {
                self.back_page_idx -= 1;
                let p = self.back_page_idx;
                if let Some(page_arc) = page_opt
                    && page_arc.present != 0
                {
                    let page = Arc::make_mut(page_arc);
                    self.back_slots = Some((p << PAGE_SHIFT, PAGE_SIZE, page.slots.iter_mut()));
                }
                continue;
            }
            if let Some((base, offset, ref mut slots)) = self.front_slots {
                while let Some(slot) = slots.next_back() {
                    let idx = base + offset + slots.len();
                    if let Some(arc) = slot {
                        self.remaining -= 1;
                        return Some((K::key_ref(idx), Arc::make_mut(arc)));
                    }
                }
                self.front_slots = None;
            }
            return None;
        }
    }
}

impl<K: CowKey, V: Clone> ExactSizeIterator for CowMapIterMut<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.remaining
    }
}

impl<K: CowKey, V: Clone> CowMap<K, V> {
    #[inline]
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if let Some(idx) = key.dense_index() {
            let p = idx >> PAGE_SHIFT;
            let s = idx & (PAGE_SIZE - 1);
            if !self.contains_key(key) {
                return None;
            }
            let inner = Arc::make_mut(&mut self.0);
            let page = Arc::make_mut(inner.pages[p].as_mut()?);
            page.slots[s].as_mut().map(Arc::make_mut)
        } else {
            if Arc::strong_count(&self.0) > 1 && !self.0.overflow.contains_key(key) {
                return None;
            }
            Arc::make_mut(&mut self.0)
                .overflow
                .get_mut(key)
                .map(Arc::make_mut)
        }
    }

    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let inner = Arc::make_mut(&mut self.0);
        if let Some(idx) = key.dense_index() {
            let p = idx >> PAGE_SHIFT;
            let s = idx & (PAGE_SIZE - 1);
            if inner.pages.len() <= p {
                inner.pages.resize_with(p + 1, || None);
            }
            let page_arc = inner.pages[p].get_or_insert_with(|| Arc::new(Page::new()));
            let page = Arc::make_mut(page_arc);
            let prev = page.slots[s].replace(Arc::new(value));
            if prev.is_none() {
                page.present |= 1u64 << s;
                inner.len += 1;
            }
            prev.map(Arc::unwrap_or_clone)
        } else {
            let prev = inner.overflow.insert(key, Arc::new(value));
            if prev.is_none() {
                inner.len += 1;
            }
            prev.map(Arc::unwrap_or_clone)
        }
    }

    #[inline]
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if !self.contains_key(key) {
            return None;
        }
        let inner = Arc::make_mut(&mut self.0);
        if let Some(idx) = key.dense_index() {
            let p = idx >> PAGE_SHIFT;
            let s = idx & (PAGE_SIZE - 1);
            let page_arc = inner.pages[p].as_mut()?;
            let page = Arc::make_mut(page_arc);
            let prev = page.slots[s].take()?;
            page.present &= !(1u64 << s);
            inner.len -= 1;
            if page.present == 0 {
                inner.pages[p] = None;
                inner.trim_pages();
            }
            Some(Arc::unwrap_or_clone(prev))
        } else {
            let prev = inner.overflow.remove(key)?;
            inner.len -= 1;
            Some(Arc::unwrap_or_clone(prev))
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        if self.0.len != 0 {
            if Arc::strong_count(&self.0) == 1 {
                let inner = Arc::make_mut(&mut self.0);
                inner.pages.clear();
                inner.overflow.clear();
                inner.len = 0;
            } else {
                self.0 = Arc::new(CowMapInner::new());
            }
        }
    }

    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        if self.0.len == 0 {
            return;
        }
        let inner = Arc::make_mut(&mut self.0);
        for (p, page_opt) in inner.pages.iter_mut().enumerate() {
            if let Some(page_arc) = page_opt
                && page_arc.present != 0
            {
                let page = Arc::make_mut(page_arc);
                let mut mask = page.present;
                while mask != 0 {
                    let s = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let idx = (p << PAGE_SHIFT) | s;
                    let keep = {
                        let arc = page.slots[s].as_mut().expect("present bit matches slot");
                        f(K::key_ref(idx), Arc::make_mut(arc))
                    };
                    if !keep {
                        page.slots[s] = None;
                        page.present &= !(1u64 << s);
                        inner.len -= 1;
                    }
                }
                if page.present == 0 {
                    *page_opt = None;
                }
            }
        }
        inner.trim_pages();
        let before = inner.overflow.len();
        inner.overflow.retain(|k, v| f(k, Arc::make_mut(v)));
        inner.len -= before - inner.overflow.len();
    }

    #[inline]
    pub fn iter_mut(&mut self) -> CowMapIterMut<'_, K, V> {
        let inner = Arc::make_mut(&mut self.0);
        let back_page_idx = inner.pages.len();
        CowMapIterMut {
            pages: inner.pages.iter_mut(),
            front_page_idx: 0,
            back_page_idx,
            front_slots: None,
            back_slots: None,
            overflow: inner.overflow.iter_mut(),
            remaining: inner.len,
        }
    }

    #[inline]
    pub fn values_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut V> + ExactSizeIterator {
        self.iter_mut().map(|(_, v)| v)
    }

    #[inline]
    pub fn entry(&mut self, key: K) -> CowEntry<'_, K, V> {
        if let Some(idx) = key.dense_index() {
            CowEntry::Dense {
                map: self,
                index: idx,
            }
        } else {
            let inner = Arc::make_mut(&mut self.0);
            CowEntry::Overflow {
                len: &mut inner.len,
                entry: inner.overflow.entry(key),
            }
        }
    }
}

/// Entry handle returned by [`CowMap::entry`].
pub enum CowEntry<'a, K: 'a, V: 'a> {
    Dense {
        map: &'a mut CowMap<K, V>,
        index: usize,
    },
    Overflow {
        len: &'a mut usize,
        entry: Entry<'a, K, Arc<V>>,
    },
}

impl<'a, K: CowKey, V: Clone> CowEntry<'a, K, V> {
    #[inline]
    pub fn or_insert(self, default: V) -> &'a mut V {
        self.or_insert_with(|| default)
    }

    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        match self {
            CowEntry::Dense { map, index } => {
                let p = index >> PAGE_SHIFT;
                let s = index & (PAGE_SIZE - 1);
                let inner = Arc::make_mut(&mut map.0);
                if inner.pages.len() <= p {
                    inner.pages.resize_with(p + 1, || None);
                }
                let page_arc = inner.pages[p].get_or_insert_with(|| Arc::new(Page::new()));
                let page = Arc::make_mut(page_arc);
                if page.slots[s].is_none() {
                    page.slots[s] = Some(Arc::new(default()));
                    page.present |= 1u64 << s;
                    inner.len += 1;
                }
                Arc::make_mut(page.slots[s].as_mut().expect("just inserted"))
            }
            CowEntry::Overflow { len, entry } => match entry {
                Entry::Occupied(o) => Arc::make_mut(o.into_mut()),
                Entry::Vacant(v) => {
                    *len += 1;
                    Arc::make_mut(v.insert(Arc::new(default())))
                }
            },
        }
    }

    #[inline]
    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

impl<K, V> Clone for CowMap<K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<K, V> Default for CowMap<K, V> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<K: CowKey, V> Index<&K> for CowMap<K, V> {
    type Output = V;

    #[inline]
    fn index(&self, key: &K) -> &Self::Output {
        self.get(key).expect("no entry found for key")
    }
}

impl<'a, K: CowKey, V> IntoIterator for &'a CowMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = CowMapIter<'a, K, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, K: CowKey, V: Clone> IntoIterator for &'a mut CowMap<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = CowMapIterMut<'a, K, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<K: CowKey, V> CowMapInner<K, V> {
    fn insert_owned(&mut self, key: K, value: V) {
        if let Some(idx) = key.dense_index() {
            let p = idx >> PAGE_SHIFT;
            let s = idx & (PAGE_SIZE - 1);
            if self.pages.len() <= p {
                self.pages.resize_with(p + 1, || None);
            }
            let page_arc = self.pages[p].get_or_insert_with(|| Arc::new(Page::new()));
            let page = Arc::get_mut(page_arc).expect("uniquely owned during construction");
            if page.slots[s].replace(Arc::new(value)).is_none() {
                page.present |= 1u64 << s;
                self.len += 1;
            }
        } else if self.overflow.insert(key, Arc::new(value)).is_none() {
            self.len += 1;
        }
    }
}

impl<K: CowKey, V> From<BTreeMap<K, V>> for CowMap<K, V> {
    fn from(map: BTreeMap<K, V>) -> Self {
        let mut inner = CowMapInner::new();
        for (k, v) in map {
            inner.insert_owned(k, v);
        }
        Self(Arc::new(inner))
    }
}

impl<K: CowKey, V, const N: usize> From<[(K, V); N]> for CowMap<K, V> {
    fn from(items: [(K, V); N]) -> Self {
        let mut inner = CowMapInner::new();
        for (k, v) in items {
            inner.insert_owned(k, v);
        }
        Self(Arc::new(inner))
    }
}

impl<K: CowKey, V> FromIterator<(K, V)> for CowMap<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut inner = CowMapInner::new();
        for (k, v) in iter {
            inner.insert_owned(k, v);
        }
        Self(Arc::new(inner))
    }
}

impl<K: CowKey, V: Clone> Extend<(K, V)> for CowMap<K, V> {
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<K: CowKey, V: PartialEq> PartialEq for CowMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.0.len == other.0.len && self.iter().eq(other.iter())
    }
}

impl<K: CowKey, V: Eq> Eq for CowMap<K, V> {}

impl<K: CowKey + fmt::Debug, V: fmt::Debug> fmt::Debug for CowMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: CowKey + Serialize, V: Serialize> Serialize for CowMap<K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len))?;
        for (k, v) in self.iter() {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de, K: CowKey + Deserialize<'de>, V: Deserialize<'de>> Deserialize<'de> for CowMap<K, V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor<K, V>(std::marker::PhantomData<(K, V)>);

        impl<'de, K: CowKey + Deserialize<'de>, V: Deserialize<'de>> serde::de::Visitor<'de>
            for Visitor<K, V>
        {
            type Value = CowMap<K, V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut inner = CowMapInner::new();
                while let Some((k, v)) = access.next_entry::<K, V>()? {
                    inner.insert_owned(k, v);
                }
                Ok(CowMap(Arc::new(inner)))
            }
        }

        deserializer.deserialize_map(Visitor(std::marker::PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cow_box_isolates_clones_and_matches_plain_postcard_bytes() {
        let mut a = CowBox::new(vec![1i32, 2, 3]);
        let b = a.clone();
        assert_eq!(a, b);
        assert!(a.ptr_eq(&b));

        a.push(4);
        assert_ne!(a, b);
        assert_eq!(*b, vec![1, 2, 3]);
        assert_eq!(*a, vec![1, 2, 3, 4]);

        let raw = vec![1i32, 2, 3, 4];
        let encoded_cow = postcard::to_allocvec(&a).expect("encode CowBox");
        let encoded_raw = postcard::to_allocvec(&raw).expect("encode Vec");
        assert_eq!(encoded_cow, encoded_raw);
        let decoded: CowBox<Vec<i32>> = postcard::from_bytes(&encoded_raw).expect("decode CowBox");
        assert_eq!(decoded, a);
    }

    #[test]
    fn cow_map_isolates_clones_and_matches_btree_postcard_bytes() {
        let mut map: CowMap<u16, Vec<i16>> =
            CowMap::from([(1, vec![10, 20]), (2, vec![30]), (5000, vec![99])]);
        let snapshot = map.clone();
        assert_eq!(map, snapshot);

        // Missing key while shared must not trigger outer clone.
        assert!(map.get_mut(&99).is_none());
        assert!(map.remove(&99).is_none());
        assert!(map.get_mut(&9999).is_none());
        assert!(map.remove(&9999).is_none());
        assert!(map.ptr_eq(&snapshot));

        // Mutating one entry detaches only that page & entry; untouched entries stay shared.
        map.get_mut(&1).unwrap().push(25);
        assert_ne!(map, snapshot);
        assert_eq!(snapshot.get(&1).unwrap(), &vec![10, 20]);
        assert_eq!(map.get(&1).unwrap(), &vec![10, 20, 25]);
        assert!(Arc::ptr_eq(
            map.0.pages[0].as_ref().unwrap().slots[2].as_ref().unwrap(),
            snapshot.0.pages[0].as_ref().unwrap().slots[2]
                .as_ref()
                .unwrap()
        ));

        // Unshared fast paths.
        drop(snapshot);
        assert!(map.get_mut(&99).is_none());
        assert_eq!(map.remove(&2), Some(vec![30]));
        assert_eq!(map.len(), 2);

        // Wire compatibility with plain BTreeMap (both dense and overflow keys).
        let mut plain = BTreeMap::new();
        plain.insert(1u16, vec![10i16, 20, 25]);
        plain.insert(5000u16, vec![99i16]);
        let encoded_cow = postcard::to_allocvec(&map).expect("encode CowMap");
        let encoded_plain = postcard::to_allocvec(&plain).expect("encode BTreeMap");
        assert_eq!(encoded_cow, encoded_plain);
        let decoded: CowMap<u16, Vec<i16>> =
            postcard::from_bytes(&encoded_plain).expect("decode CowMap");
        assert_eq!(decoded, map);
    }

    #[test]
    fn cow_map_iterators_double_ended_range_and_next_key_from() {
        let keys = [0u16, 1, 63, 64, 127, 250, 4095, 4096, 4097, 65535];
        let mut map: CowMap<u16, u16> = keys.iter().map(|&k| (k, k.wrapping_mul(3))).collect();
        let expected: Vec<(u16, u16)> = keys.iter().map(|&k| (k, k.wrapping_mul(3))).collect();

        // Forward and reverse iteration match exact ascending/descending order.
        let fwd: Vec<_> = map.iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(fwd, expected);
        let rev: Vec<_> = map.iter().rev().map(|(&k, &v)| (k, v)).collect();
        let mut exp_rev = expected.clone();
        exp_rev.reverse();
        assert_eq!(rev, exp_rev);

        // Interleaved next / next_back meeting in the same page.
        let mut it = map.iter();
        let mut front = Vec::new();
        let mut back = Vec::new();
        while let Some((&k, &v)) = it.next() {
            front.push((k, v));
            if let Some((&bk, &bv)) = it.next_back() {
                back.push((bk, bv));
            }
        }
        back.reverse();
        front.extend(back);
        assert_eq!(front, expected);

        // Interleaved iter_mut next / next_back.
        {
            let mut it = map.iter_mut();
            let mut seen = Vec::new();
            while let Some((&k, v)) = it.next() {
                *v = v.wrapping_add(1);
                seen.push(k);
                if let Some((&bk, bv)) = it.next_back() {
                    *bv = bv.wrapping_add(1);
                    seen.push(bk);
                }
            }
            seen.sort_unstable();
            assert_eq!(seen, keys);
        }

        // next_key_from & range across dense/overflow boundary.
        assert_eq!(map.next_key_from(0), Some(0));
        assert_eq!(map.next_key_from(2), Some(63));
        assert_eq!(map.next_key_from(64), Some(64));
        assert_eq!(map.next_key_from(251), Some(4095));
        assert_eq!(map.next_key_from(4096), Some(4096));
        assert_eq!(map.next_key_from(5000), Some(65535));
        let r: Vec<u16> = map.range(63..=4096).map(|(&k, _)| k).collect();
        assert_eq!(r, vec![63, 64, 127, 250, 4095, 4096]);
        assert_eq!(map.first_key_value().map(|(&k, _)| k), Some(0));
        assert_eq!(map.last_key_value().map(|(&k, _)| k), Some(65535));

        // Retain & page trimming.
        map.retain(|&k, _| k == 1 || k == 4097);
        assert_eq!(map.keys().copied().collect::<Vec<_>>(), vec![1, 4097]);
        assert_eq!(map.len(), 2);
        assert_eq!(map.0.pages.len(), 1);
    }

    #[test]
    fn cow_wrappers_match_plain_nan_inequality() {
        let box_nan = CowBox::new(f32::NAN);
        let box_clone = box_nan.clone();
        assert_ne!(box_nan, box_clone);

        let map_nan: CowMap<u16, f32> = CowMap::from([(1, f32::NAN)]);
        let map_clone = map_nan.clone();
        assert_ne!(map_nan, map_clone);
    }
}
