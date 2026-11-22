//! Copy-on-write containers backed by [`Arc`] with wire-compatible `serde`
//! serialization.
//!
//! `World::clone` runs before every interactive tick (`Lot::step_in` when
//! `asks = true`) and before speculative player commands (`buy_move`,
//! `pie_menu`, `grab`, `drop`, etc.). Wrapping the world's large, mostly-static
//! maps and tables in [`CowMap`] and [`CowBox`] makes cloning `O(1)` while
//! preserving bit-for-bit identical `postcard` serialization and snapshot
//! hashes.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::collections::btree_map::{self, Entry};
use std::fmt;
use std::ops::{Deref, DerefMut, Index};
use std::sync::Arc;

/// A copy-on-write single value backed by [`Arc`].
///
/// Serializes and deserializes identically to `T`.
pub struct CowBox<T>(Arc<T>);

impl<T> CowBox<T> {
    #[inline]
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
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

/// A two-level copy-on-write ordered map (`Arc<BTreeMap<K, Arc<V>>>`).
///
/// Cloning is `O(1)`. Mutating one entry clones only the outer map index (once
/// per clone generation) and the single mutated value `V`, leaving all other
/// values shared. Serializes and deserializes identically to `BTreeMap<K, V>`.
pub struct CowMap<K, V>(Arc<BTreeMap<K, Arc<V>>>);

impl<K, V> CowMap<K, V> {
    #[inline]
    pub fn new() -> Self {
        Self(Arc::new(BTreeMap::new()))
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn keys(&self) -> btree_map::Keys<'_, K, Arc<V>> {
        self.0.keys()
    }

    #[inline]
    pub fn iter(&self) -> CowMapIter<'_, K, V> {
        CowMapIter(self.0.iter())
    }

    #[inline]
    pub fn values(&self) -> CowMapValues<'_, K, V> {
        CowMapValues(self.0.values())
    }
}

/// Borrowing iterator over `(&K, &V)` entries in a [`CowMap`].
pub struct CowMapIter<'a, K, V>(btree_map::Iter<'a, K, Arc<V>>);

impl<'a, K, V> Iterator for CowMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k, v.as_ref()))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K, V> DoubleEndedIterator for CowMapIter<'_, K, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (k, v.as_ref()))
    }
}

impl<K, V> ExactSizeIterator for CowMapIter<'_, K, V> {
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<K, V> Clone for CowMapIter<'_, K, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Borrowing iterator over `&V` values in a [`CowMap`].
pub struct CowMapValues<'a, K, V>(btree_map::Values<'a, K, Arc<V>>);

impl<'a, K, V> Iterator for CowMapValues<'a, K, V> {
    type Item = &'a V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Arc::as_ref)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K, V> DoubleEndedIterator for CowMapValues<'_, K, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(Arc::as_ref)
    }
}

impl<K, V> ExactSizeIterator for CowMapValues<'_, K, V> {
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

impl<K: Ord, V> CowMap<K, V> {
    #[inline]
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        self.0.first_key_value().map(|(k, v)| (k, v.as_ref()))
    }

    #[inline]
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        self.0.last_key_value().map(|(k, v)| (k, v.as_ref()))
    }

    #[inline]
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.0.contains_key(key)
    }

    #[inline]
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.0.get(key).map(Arc::as_ref)
    }

    #[inline]
    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.0.get_key_value(key).map(|(k, v)| (k, v.as_ref()))
    }
}

impl<K: Ord + Clone, V: Clone> CowMap<K, V> {
    #[inline]
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        if Arc::strong_count(&self.0) > 1 && !self.0.contains_key(key) {
            return None;
        }
        Arc::make_mut(&mut self.0).get_mut(key).map(Arc::make_mut)
    }

    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        Arc::make_mut(&mut self.0)
            .insert(key, Arc::new(value))
            .map(Arc::unwrap_or_clone)
    }

    #[inline]
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        if Arc::strong_count(&self.0) > 1 && !self.0.contains_key(key) {
            return None;
        }
        Arc::make_mut(&mut self.0)
            .remove(key)
            .map(Arc::unwrap_or_clone)
    }

    #[inline]
    pub fn clear(&mut self) {
        if !self.0.is_empty() {
            if Arc::strong_count(&self.0) == 1 {
                Arc::make_mut(&mut self.0).clear();
            } else {
                self.0 = Arc::new(BTreeMap::new());
            }
        }
    }

    #[inline]
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        Arc::make_mut(&mut self.0).retain(|k, v| f(k, Arc::make_mut(v)));
    }

    #[inline]
    pub fn iter_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = (&K, &mut V)> + ExactSizeIterator {
        Arc::make_mut(&mut self.0)
            .iter_mut()
            .map(|(k, v)| (k, Arc::make_mut(v)))
    }

    #[inline]
    pub fn values_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut V> + ExactSizeIterator {
        Arc::make_mut(&mut self.0).values_mut().map(Arc::make_mut)
    }

    #[inline]
    pub fn entry(&mut self, key: K) -> CowEntry<'_, K, V> {
        CowEntry(Arc::make_mut(&mut self.0).entry(key))
    }
}

/// Entry handle returned by [`CowMap::entry`].
pub struct CowEntry<'a, K: 'a, V: 'a>(Entry<'a, K, Arc<V>>);

impl<'a, K: Ord, V: Clone> CowEntry<'a, K, V> {
    #[inline]
    pub fn or_insert(self, default: V) -> &'a mut V {
        Arc::make_mut(self.0.or_insert_with(|| Arc::new(default)))
    }

    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'a mut V {
        Arc::make_mut(self.0.or_insert_with(|| Arc::new(default())))
    }

    #[inline]
    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        Arc::make_mut(self.0.or_insert_with(|| Arc::new(V::default())))
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

impl<K: Ord, Q, V> Index<&Q> for CowMap<K, V>
where
    K: Borrow<Q>,
    Q: Ord + ?Sized,
{
    type Output = V;

    #[inline]
    fn index(&self, key: &Q) -> &Self::Output {
        self.get(key).expect("no entry found for key")
    }
}

impl<'a, K, V> IntoIterator for &'a CowMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = CowMapIter<'a, K, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, K: Ord + Clone, V: Clone> IntoIterator for &'a mut CowMap<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = std::iter::Map<
        btree_map::IterMut<'a, K, Arc<V>>,
        fn((&'a K, &'a mut Arc<V>)) -> (&'a K, &'a mut V),
    >;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Arc::make_mut(&mut self.0)
            .iter_mut()
            .map(|(k, v)| (k, Arc::make_mut(v)))
    }
}

impl<K: Ord, V> From<BTreeMap<K, V>> for CowMap<K, V> {
    fn from(map: BTreeMap<K, V>) -> Self {
        Self(Arc::new(
            map.into_iter().map(|(k, v)| (k, Arc::new(v))).collect(),
        ))
    }
}

impl<K: Ord, V, const N: usize> From<[(K, V); N]> for CowMap<K, V> {
    fn from(items: [(K, V); N]) -> Self {
        Self(Arc::new(
            items.into_iter().map(|(k, v)| (k, Arc::new(v))).collect(),
        ))
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for CowMap<K, V> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self(Arc::new(
            iter.into_iter().map(|(k, v)| (k, Arc::new(v))).collect(),
        ))
    }
}

impl<K: Ord + Clone, V: Clone> Extend<(K, V)> for CowMap<K, V> {
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        let map = Arc::make_mut(&mut self.0);
        for (k, v) in iter {
            map.insert(k, Arc::new(v));
        }
    }
}

impl<K: PartialEq, V: PartialEq> PartialEq for CowMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.iter().eq(other.iter())
    }
}

impl<K: Eq, V: Eq> Eq for CowMap<K, V> {}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for CowMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Serialize, V: Serialize> Serialize for CowMap<K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter())
    }
}

impl<'de, K: Ord + Deserialize<'de>, V: Deserialize<'de>> Deserialize<'de> for CowMap<K, V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        BTreeMap::<K, V>::deserialize(deserializer).map(Self::from)
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
        assert!(Arc::ptr_eq(&a.0, &b.0));

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
        let mut map: CowMap<u16, Vec<i16>> = CowMap::from([(1, vec![10, 20]), (2, vec![30])]);
        let snapshot = map.clone();
        assert_eq!(map, snapshot);

        // Missing key while shared must not trigger outer clone.
        assert!(map.get_mut(&99).is_none());
        assert!(map.remove(&99).is_none());
        assert!(Arc::ptr_eq(&map.0, &snapshot.0));

        // Mutating one entry detaches only that entry; untouched entries stay shared.
        map.get_mut(&1).unwrap().push(25);
        assert_ne!(map, snapshot);
        assert_eq!(snapshot.get(&1).unwrap(), &vec![10, 20]);
        assert_eq!(map.get(&1).unwrap(), &vec![10, 20, 25]);
        assert!(Arc::ptr_eq(
            map.0.get(&2).unwrap(),
            snapshot.0.get(&2).unwrap()
        ));

        // Unshared fast paths.
        drop(snapshot);
        assert!(map.get_mut(&99).is_none());
        assert_eq!(map.remove(&2), Some(vec![30]));
        assert_eq!(map.len(), 1);

        // Wire compatibility with plain BTreeMap.
        let mut plain = BTreeMap::new();
        plain.insert(1u16, vec![10i16, 20, 25]);
        let encoded_cow = postcard::to_allocvec(&map).expect("encode CowMap");
        let encoded_plain = postcard::to_allocvec(&plain).expect("encode BTreeMap");
        assert_eq!(encoded_cow, encoded_plain);
        let decoded: CowMap<u16, Vec<i16>> =
            postcard::from_bytes(&encoded_plain).expect("decode CowMap");
        assert_eq!(decoded, map);
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
