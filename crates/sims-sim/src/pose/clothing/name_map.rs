//! Original appearance-name map: 5a1f67 / 5a1fb1 / 5a330e / 5a3292;
//! skeleton/animation maps: 5a1ded / 5a1eaa / 5a1e37 / 5a1ef4, and
//! directory file map: 5bae22 / 5baec0 / 5bb5f4 / 5bb473 / 5bbe3d.
//! Input insertion order remains explicit. This does not infer file precedence.
use super::generator::fixed;
use std::collections::BTreeMap;

pub(crate) const PRIMES: [u32; 28] = [
    53, 97, 193, 389, 769, 1543, 3079, 6151, 12289, 24593, 49157, 98317, 196613, 393241, 786433,
    1572869, 3145739, 6291469, 12582917, 25165843, 50331653, 100663319, 201326611, 402653189,
    805306457, 1610612741, 3221225473, 4294967291,
];

/// 5b8ef9 reads a C string and sign-extends each byte; equality separately uses
/// the original counted string and does not fold case.
pub fn hash(name: &[u8]) -> u32 {
    let mut value = 0_u32;
    for &byte in name.iter().take_while(|&&b| b != 0) {
        value = value
            .wrapping_add(byte as i8 as i32 as u32)
            .wrapping_mul(0x401);
        value ^= value >> 6;
    }
    value = value.wrapping_mul(9);
    (value ^ (value >> 11)).wrapping_mul(0x8001)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    buckets: Vec<Vec<Vec<u8>>>,
    length: usize,
}
impl Default for Names {
    fn default() -> Self {
        // Original constructor requests 100; 402a88 selects the next prime.
        Self {
            buckets: vec![vec![]; 193],
            length: 0,
        }
    }
}
impl Names {
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }
    pub fn contains(&self, name: &[u8]) -> bool {
        self.buckets[hash(name) as usize % self.buckets.len()]
            .iter()
            .any(|value| value == name)
    }

    pub fn insert(&mut self, name: Vec<u8>) -> bool {
        let wanted = self.length + 1;
        if self.buckets.len() < wanted {
            let count = PRIMES
                .iter()
                .copied()
                .find(|&n| n as usize >= wanted)
                .unwrap_or(PRIMES[27]) as usize;
            if self.buckets.len() < count {
                let old = std::mem::replace(&mut self.buckets, vec![vec![]; count]);
                // Visit old buckets/head links, then prepend into new buckets.
                for name in old.into_iter().flatten() {
                    self.buckets[hash(&name) as usize % count].insert(0, name);
                }
            }
        }
        // Growth happens even when this key is already present.
        let count = self.buckets.len();
        let bucket = &mut self.buckets[hash(&name) as usize % count];
        if bucket.contains(&name) {
            return false;
        }
        bucket.insert(0, name);
        self.length += 1;
        true
    }

    /// 5807c2 / 40fdf7: ascending buckets, then each bucket's head chain.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.buckets.iter().flatten().map(Vec::as_slice)
    }

    /// Locate the current node in the live table, then follow its collision
    /// chain or the next occupied bucket. Recompute the bucket after growth;
    /// retaining an ordinal would change enumeration after an insertion.
    fn successor(&self, current: &[u8]) -> Result<Option<&[u8]>, ()> {
        let bucket = hash(current) as usize % self.buckets.len();
        let chain = &self.buckets[bucket];
        let index = chain.iter().position(|name| name == current).ok_or(())?;
        Ok(chain[index + 1..]
            .first()
            .or_else(|| self.buckets[bucket + 1..].iter().find_map(|b| b.first()))
            .map(Vec::as_slice))
    }

    /// 409255 excludes the exact lowercase prefix, before CFString64 copying.
    pub fn bodies(&self) -> Vec<Vec<u8>> {
        self.iter()
            .filter(|name| !name.starts_with(b"suit-"))
            .map(|name| fixed(name, 63).to_vec())
            .collect()
    }
}

/// The file manager uses the same bucket growth/iteration algorithm, but callers
/// normalize the key and check existence *before* requesting growth. Payloads
/// belong to the first insertion. Loose-file updates/overrides are separate
/// file-manager operations and are not implemented by this map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files<T> {
    names: Names,
    values: BTreeMap<Vec<u8>, T>,
}
impl<T> Default for Files<T> {
    fn default() -> Self {
        Self {
            names: Names::default(),
            values: BTreeMap::new(),
        }
    }
}
impl<T> Files<T> {
    /// 5b96c8, using the captured C locale. Filesystem names cannot contain NUL.
    pub fn key(name: &[u8]) -> Result<Vec<u8>, String> {
        if name.contains(&0) {
            return Err("native file-map name contains an embedded NUL".into());
        }
        Ok(name
            .iter()
            .map(|&b| {
                if b == b'/' {
                    b'\\'
                } else {
                    b.to_ascii_lowercase()
                }
            })
            .collect())
    }
    pub fn insert(&mut self, name: &[u8], value: T) -> Result<bool, String> {
        let key = Self::key(name)?;
        if self.names.contains(&key) {
            return Ok(false);
        }
        self.names.insert(key.clone());
        self.values.insert(key, value);
        Ok(true)
    }
    pub fn get(&self, name: &[u8]) -> Result<Option<&T>, String> {
        Ok(self.values.get(&Self::key(name)?))
    }
    /// Update metadata without removing/reinserting its hash-table node.
    pub fn get_mut(&mut self, name: &[u8]) -> Result<Option<&mut T>, String> {
        Ok(self.values.get_mut(&Self::key(name)?))
    }
    /// Replace payloads while retaining buckets and collision chains exactly.
    /// Reinserting iteration results would reverse chains and can change later
    /// enumeration, even when every key and value remains present.
    pub fn try_map<U, E>(
        &self,
        mut map: impl FnMut(&[u8], &T) -> Result<U, E>,
    ) -> Result<Files<U>, E> {
        let mut values = BTreeMap::new();
        for (key, value) in self.iter() {
            values.insert(key.to_vec(), map(key, value)?);
        }
        Ok(Files {
            names: self.names.clone(),
            values,
        })
    }
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
    pub fn bucket_count(&self) -> usize {
        self.names.bucket_count()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &T)> {
        self.names.iter().map(|key| (key, &self.values[key]))
    }

    /// `current` is a previously enumerated, already-normalized node key.
    /// Missing current and end-of-iteration remain distinct. Payloads are read
    /// from the live map, including metadata changed since the previous step.
    pub(crate) fn successor(&self, current: &[u8]) -> Result<Option<(&[u8], &T)>, ()> {
        Ok(self
            .names
            .successor(current)?
            .map(|key| (key, &self.values[key])))
    }
}
