//! The original's registry of sound objects, kept only for its order.
//!
//! *(code)* The sound manager keeps every patch and track object in one
//! hash map at manager +0x80, keyed by ID (0x57cdd5). Each `[Patch]` and
//! `[Track]` line of a module's HOT file registers one (0x57b33b and
//! 0x57b676, the handlers the HOT reader calls for those sections at
//! 0x57affb and 0x57b02c). So does each station patch, when the stations
//! are built after the sound tables (0x574f0e). An ID registered again
//! replaces the object in its node, which stays where it was.
//!
//! The map is the SGI `hash_map` the original was built with:
//! - It starts with 193 buckets (0x57c3ff, 0x580a08).
//! - Each insert first grows the map for one more entry (0x5813bd calls
//!   0x5823a5 with the count plus one). It grows when that is more than
//!   the bucket count: to the first of [`PRIMES`] (0x666474) not below it
//!   (0x402a88).
//! - A new node goes at the head of its bucket, the ID modulo the bucket
//!   count.
//! - Growing takes the old buckets in order and each one's nodes from the
//!   head, and puts each at the head of its new bucket (0x582423 to
//!   0x582456).
//!
//! The unpause walks the map bucket by bucket, each from its head
//! (0x57cd56, 0x5807c2, 0x576be5). That is [`Registry::order`].

/// The bucket counts the map grows through (0x666474).
pub const PRIMES: [u32; 28] = [
    53,
    97,
    193,
    389,
    769,
    1_543,
    3_079,
    6_151,
    12_289,
    24_593,
    49_157,
    98_317,
    196_613,
    393_241,
    786_433,
    1_572_869,
    3_145_739,
    6_291_469,
    12_582_917,
    25_165_843,
    50_331_653,
    100_663_319,
    201_326_611,
    402_653_189,
    805_306_457,
    1_610_612_741,
    3_221_225_473,
    4_294_967_291,
];

/// The bucket count the map starts with (0x57c3ff, 0x580a08).
pub const FIRST_BUCKETS: u32 = 193;

/// The registry's IDs, in the original's buckets and chains.
#[derive(Debug, Clone)]
pub struct Registry {
    /// Each bucket's first node.
    heads: Vec<Option<usize>>,
    /// Each node's ID and the next node in its bucket.
    nodes: Vec<(u32, Option<usize>)>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// An empty registry of [`FIRST_BUCKETS`] buckets.
    pub fn new() -> Self {
        Self {
            heads: vec![None; FIRST_BUCKETS as usize],
            nodes: Vec::new(),
        }
    }

    /// Registers an ID. An ID already registered keeps its place.
    pub fn insert(&mut self, id: u32) {
        self.grow(self.nodes.len() + 1);
        let bucket = self.bucket(id);
        let mut node = self.heads[bucket];
        while let Some(at) = node {
            if self.nodes[at].0 == id {
                return;
            }
            node = self.nodes[at].1;
        }
        self.nodes.push((id, self.heads[bucket]));
        self.heads[bucket] = Some(self.nodes.len() - 1);
    }

    /// How many IDs are registered.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether no ID is registered.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// How many buckets the map has.
    pub fn buckets(&self) -> usize {
        self.heads.len()
    }

    /// The IDs in the order the unpause walks them: bucket by bucket,
    /// each from its head.
    pub fn order(&self) -> impl Iterator<Item = u32> + '_ {
        self.heads.iter().flat_map(move |&head| {
            std::iter::successors(head, move |&at| self.nodes[at].1).map(move |at| self.nodes[at].0)
        })
    }

    fn bucket(&self, id: u32) -> usize {
        // The bucket count always fits a u32 (it is one of PRIMES).
        (id % self.heads.len() as u32) as usize
    }

    /// 0x5823a5: grows the map to hold `wanted` IDs.
    fn grow(&mut self, wanted: usize) {
        let old = self.heads.len();
        if wanted <= old {
            return;
        }
        let wanted = u32::try_from(wanted).unwrap_or(u32::MAX);
        let count = PRIMES
            .iter()
            .copied()
            .find(|&p| p >= wanted)
            .unwrap_or(PRIMES[PRIMES.len() - 1]) as usize;
        if count <= old {
            return;
        }
        let mut heads = vec![None; count];
        for bucket in 0..old {
            while let Some(at) = self.heads[bucket] {
                self.heads[bucket] = self.nodes[at].1;
                let new = (self.nodes[at].0 % count as u32) as usize;
                self.nodes[at].1 = heads[new];
                heads[new] = Some(at);
            }
        }
        self.heads = heads;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_walk_by_bucket_then_newest_first() {
        let mut registry = Registry::new();
        for id in [1005, 719, 5, 198, 391] {
            registry.insert(id);
        }
        // 5, 198 and 391 share bucket 5; 1005 is in 40 and 719 in 140.
        assert_eq!(
            registry.order().collect::<Vec<_>>(),
            [391, 198, 5, 1005, 719]
        );
    }

    #[test]
    fn an_id_registered_again_keeps_its_place() {
        let mut registry = Registry::new();
        for id in [5, 198, 5] {
            registry.insert(id);
        }
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.order().collect::<Vec<_>>(), [198, 5]);
    }

    #[test]
    fn the_map_grows_to_the_next_prime_and_turns_chains_around() {
        let mut registry = Registry::new();
        // 0 and 75,077 (193 times 389) share bucket 0 of 193, 75,077 at
        // the head, and bucket 0 of 389. 1 to 191 fill the rest.
        registry.insert(0);
        registry.insert(75_077);
        for id in 1..=191 {
            registry.insert(id);
        }
        assert_eq!(registry.buckets(), 193);
        assert_eq!(registry.order().take(2).collect::<Vec<_>>(), [75_077, 0]);
        // The 194th ID grows it to 389 buckets. Bucket 0 is taken from
        // its head, each node put at the new head, so 0 ends first.
        registry.insert(192);
        assert_eq!(registry.buckets(), 389);
        assert_eq!(registry.order().take(2).collect::<Vec<_>>(), [0, 75_077]);
    }
}
