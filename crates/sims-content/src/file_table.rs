//! The order the original lists a directory in.
//!
//! The original keeps each directory's entries (files, subdirectories and
//! the members of any FAR archive in it) in a hash table keyed by name, and
//! its object scan walks that table (0x5ba9bd, 0x5baac8), not the file
//! system. So the order objects register in depends on the hash of each
//! name, on how many entries the directory has, and, for names that land in
//! the same bucket, on the order they were inserted.
//!
//! Ported from the old prototype's `name_map` (legacy
//! crates/sims-sim/src/pose/clothing/name_map.rs), which recovered the hash
//! (0x5b8ef9), the bucket counts (0x402a88) and the growth rule (0x5bb5f4).

/// Bucket counts the table grows through.
const PRIMES: [usize; 28] = [
    53, 97, 193, 389, 769, 1543, 3079, 6151, 12289, 24593, 49157, 98317, 196613, 393241, 786433,
    1572869, 3145739, 6291469, 12582917, 25165843, 50331653, 100663319, 201326611, 402653189,
    805306457, 1610612741, 3221225473, 4294967291,
];

/// The table key for a name: ASCII lower case, with `/` as `\` (0x5b96c8).
pub(crate) fn key(name: &[u8]) -> Vec<u8> {
    name.iter()
        .map(|&b| match b {
            b'/' => b'\\',
            _ => b.to_ascii_lowercase(),
        })
        .collect()
}

/// The name hash (0x5b8ef9). Each byte is sign-extended before it is added.
pub(crate) fn hash(name: &[u8]) -> u32 {
    let mut value = 0u32;
    for &byte in name {
        value = value
            .wrapping_add(byte as i8 as i32 as u32)
            .wrapping_mul(0x401);
        value ^= value >> 6;
    }
    value = value.wrapping_mul(9);
    (value ^ (value >> 11)).wrapping_mul(0x8001)
}

/// One directory's names, in the original's table layout.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    /// Each bucket's chain, head first.
    buckets: Vec<Vec<Vec<u8>>>,
    len: usize,
}

impl Default for Table {
    fn default() -> Self {
        // The original asks for 100 buckets and gets the next prime.
        Self {
            buckets: vec![Vec::new(); 193],
            len: 0,
        }
    }
}

impl Table {
    fn bucket(&self, key: &[u8]) -> usize {
        hash(key) as usize % self.buckets.len()
    }

    pub(crate) fn contains(&self, key: &[u8]) -> bool {
        self.buckets[self.bucket(key)].iter().any(|k| k == key)
    }

    /// Adds a key, which must already be in [`key`] form. Returns false,
    /// changing nothing, when it is present: the first entry of a name wins.
    pub(crate) fn insert(&mut self, key: Vec<u8>) -> bool {
        if self.contains(&key) {
            return false;
        }
        let wanted = self.len + 1;
        if self.buckets.len() < wanted {
            let count = PRIMES
                .iter()
                .copied()
                .find(|&n| n >= wanted)
                .unwrap_or(PRIMES[27]);
            let old = std::mem::replace(&mut self.buckets, vec![Vec::new(); count]);
            // Old buckets in order, each chain from its head, and every key
            // goes to the head of its new chain.
            for key in old.into_iter().flatten() {
                let bucket = self.bucket(&key);
                self.buckets[bucket].insert(0, key);
            }
        }
        let bucket = self.bucket(&key);
        self.buckets[bucket].insert(0, key);
        self.len += 1;
        true
    }

    /// The walk order: buckets in order, each chain from its head.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.buckets.iter().flatten().map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_fold_case_and_slashes() {
        assert_eq!(key(b"Sub/Chair.IFF"), b"sub\\chair.iff");
    }

    #[test]
    fn hash_matches_hand_computed_values() {
        assert_eq!(hash(b""), 0);
        // One byte: ((0 + 97) * 0x401) ^ (that >> 6), then the two final mixes.
        let mut value = 97u32.wrapping_mul(0x401);
        value ^= value >> 6;
        value = value.wrapping_mul(9);
        let expected = (value ^ (value >> 11)).wrapping_mul(0x8001);
        assert_eq!(hash(b"a"), expected);
        // Bytes above 0x7f are added as negative numbers.
        assert_ne!(hash(&[0xe9]), hash(&[0x69]));
        let mut value = (0xe9u8 as i8 as i32 as u32).wrapping_mul(0x401);
        value ^= value >> 6;
        value = value.wrapping_mul(9);
        assert_eq!(hash(&[0xe9]), (value ^ (value >> 11)).wrapping_mul(0x8001));
    }

    #[test]
    fn walks_buckets_in_order_and_newer_keys_first_within_one() {
        let mut table = Table::default();
        let names: Vec<Vec<u8>> = (0..150u32)
            .map(|n| format!("file{n}.iff").into_bytes())
            .collect();
        for name in &names {
            assert!(table.insert(name.clone()));
        }
        assert!(!table.insert(names[3].clone()));
        let walked: Vec<&[u8]> = table.iter().collect();
        assert_eq!(walked.len(), 150);
        let mut expected: Vec<(usize, usize)> = names
            .iter()
            .enumerate()
            .map(|(n, name)| (hash(name) as usize % 193, n))
            .collect();
        // Bucket ascending, then later insertions first.
        expected.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let expected: Vec<&[u8]> = expected.iter().map(|&(_, n)| &names[n][..]).collect();
        assert_eq!(walked, expected);
    }

    #[test]
    fn growing_rehashes_in_walk_order() {
        let mut table = Table::default();
        let names: Vec<Vec<u8>> = (0..194u32).map(|n| format!("n{n}").into_bytes()).collect();
        for name in &names[..193] {
            table.insert(name.clone());
        }
        let before: Vec<Vec<u8>> = table.iter().map(<[u8]>::to_vec).collect();
        // The 194th key does not fit 193 buckets: the table grows to 389.
        table.insert(names[193].clone());
        assert_eq!(table.buckets.len(), 389);
        // Keys that shared an old chain and share a new one swap order,
        // because each is pushed onto the head as the old walk reaches it.
        let mut chains = vec![Vec::new(); 389];
        for key in before {
            chains[hash(&key) as usize % 389].insert(0, key);
        }
        chains[hash(&names[193]) as usize % 389].insert(0, names[193].clone());
        let expected: Vec<Vec<u8>> = chains.into_iter().flatten().collect();
        let walked: Vec<Vec<u8>> = table.iter().map(<[u8]>::to_vec).collect();
        assert_eq!(walked, expected);
    }
}
