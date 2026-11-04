//! 43e44b/43e4da and 411bbf/411c0f/411c94. The native resource cache
//! keys by bounded basename INCLUDING extension, not by full filesystem path.
use crate::pose::clothing::name_map::{PRIMES, hash};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry<R> {
    pub key: Vec<u8>,
    pub references: u32,
    pub resource: R,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache<R> {
    buckets: Vec<Vec<Entry<R>>>,
    len: usize,
}
impl<R> Default for Cache<R> {
    fn default() -> Self {
        Self {
            buckets: (0..193).map(|_| vec![]).collect(),
            len: 0,
        }
    }
}
pub trait Host {
    type Resource: Copy;
    fn construct(&mut self) -> Result<Self::Resource, String>;
    /// 465949 receives mode 4 for People=false, otherwise mode 0.
    fn open(&mut self, resource: Self::Resource, path: &[u8], mode: u8) -> Result<(), String>;
    fn valid(&mut self, resource: Self::Resource) -> Result<bool, String>;
    fn filename(&mut self, resource: Self::Resource) -> Result<Vec<u8>, String>;
    fn close(&mut self, resource: Self::Resource) -> Result<(), String>;
    fn destroy(&mut self, resource: Self::Resource) -> Result<(), String>;
}
pub fn key(path: &[u8]) -> Result<Vec<u8>, String> {
    let (_, basename, extension) = super::parts(path)?;
    let mut key = [basename, extension].concat();
    key.truncate(31);
    Ok(key)
}
impl<R: Copy> Cache<R> {
    /// Import an observed table without reinserting keys or replaying retains.
    /// Resource identities belong to the caller's explicitly retained domain.
    pub fn from_retained(buckets: Vec<Vec<Entry<R>>>, len: usize) -> Result<Self, String> {
        if buckets.is_empty()
            || !PRIMES.iter().any(|&p| p as usize == buckets.len())
            || buckets.iter().map(Vec::len).sum::<usize>() != len
        {
            return Err("invalid retained object resource cache owner".into());
        }
        for (bucket, chain) in buckets.iter().enumerate() {
            for (i, entry) in chain.iter().enumerate() {
                if entry.key.len() > 31
                    || entry.key.contains(&0)
                    || hash(&entry.key) as usize % buckets.len() != bucket
                    || chain[..i]
                        .iter()
                        .any(|other| other.key.eq_ignore_ascii_case(&entry.key))
                {
                    return Err("invalid retained object resource cache topology".into());
                }
            }
        }
        Ok(Self { buckets, len })
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry<R>> {
        self.buckets.iter().flatten()
    }
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }
    fn location(&self, key: &[u8]) -> (usize, Option<usize>) {
        // 5b8ef9 does NOT fold case. 46f874 compares case-insensitively only
        // inside that bucket. Lowercasing the whole map would change identity.
        let bucket = hash(key) as usize % self.buckets.len();
        (
            bucket,
            self.buckets[bucket]
                .iter()
                .position(|e| e.key.eq_ignore_ascii_case(key)),
        )
    }
    fn grow(&mut self) -> Result<(), String> {
        let wanted = self
            .len
            .checked_add(1)
            .ok_or("resource cache size overflow")?;
        if wanted > self.buckets.len() {
            let count = PRIMES
                .iter()
                .copied()
                .find(|&n| n as usize >= wanted)
                .ok_or("resource cache exceeds native bucket table")?
                as usize;
            let old = std::mem::replace(&mut self.buckets, (0..count).map(|_| vec![]).collect());
            for entry in old.into_iter().flatten() {
                self.buckets[hash(&entry.key) as usize % count].insert(0, entry);
            }
        }
        Ok(())
    }
    fn retain(&mut self, resource: R, host: &mut impl Host<Resource = R>) -> Result<(), String> {
        let key = key(&host.filename(resource)?)?;
        let (bucket, existing) = self.location(&key);
        if let Some(index) = existing {
            let entry = &mut self.buckets[bucket][index];
            entry.references = entry.references.wrapping_add(1);
        } else {
            // 411d93 grows before its own second lookup. Rehash can change
            // which case-different key shares the query's bucket.
            self.grow()?;
            let (bucket, existing) = self.location(&key);
            if let Some(index) = existing {
                self.buckets[bucket][index].references = 1;
                self.buckets[bucket][index].resource = resource;
            } else {
                self.buckets[bucket].insert(
                    0,
                    Entry {
                        key,
                        references: 1,
                        resource,
                    },
                );
                self.len += 1;
            }
        }
        Ok(())
    }
    pub fn acquire(
        &mut self,
        path: &[u8],
        people: u8,
        host: &mut impl Host<Resource = R>,
    ) -> Result<Option<R>, String> {
        let key = key(path)?;
        let (bucket, existing) = self.location(&key);
        let resource = if let Some(index) = existing {
            // Existing resources skip open/status checks, even if their status
            // changed since registration. Retain uses their reported filename.
            self.buckets[bucket][index].resource
        } else {
            let resource = host.construct()?;
            host.open(resource, path, if people == 0 { 4 } else { 0 })?;
            if !host.valid(resource)? {
                host.destroy(resource)?;
                return Ok(None);
            }
            resource
        };
        self.retain(resource, host)?;
        Ok(Some(resource))
    }
    pub fn release(
        &mut self,
        resource: R,
        host: &mut impl Host<Resource = R>,
    ) -> Result<(), String> {
        let key = key(&host.filename(resource)?)?;
        let (bucket, existing) = self.location(&key);
        if let Some(index) = existing {
            let entry = &mut self.buckets[bucket][index];
            entry.references = entry.references.wrapping_sub(1);
            if entry.references == 0 {
                self.buckets[bucket].remove(index);
                self.len -= 1;
                if host.valid(resource)? {
                    host.close(resource)?;
                }
                host.destroy(resource)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod retained_tests {
    use super::*;

    #[test]
    fn import_preserves_chains_aliases_and_wrapped_references() {
        let mut buckets = vec![vec![]; 193];
        let key = b"Chair.iff".to_vec();
        buckets[hash(&key) as usize % 193].push(Entry {
            key,
            references: 0,
            resource: 7,
        });
        let key = b"Other.iff".to_vec();
        buckets[hash(&key) as usize % 193].push(Entry {
            key,
            references: u32::MAX,
            resource: 7,
        });
        let expected = buckets.clone();
        let cache = Cache::from_retained(buckets, 2).unwrap();
        assert_eq!(cache.buckets, expected);
        assert_eq!(cache.len, 2);
    }

    #[test]
    fn malformed_retained_tables_are_rejected() {
        assert!(Cache::<u32>::from_retained(vec![], 0).is_err());
        assert!(Cache::<u32>::from_retained(vec![vec![]; 194], 0).is_err());
        assert!(Cache::<u32>::from_retained(vec![vec![]; 193], 1).is_err());
        let key = b"Chair.iff".to_vec();
        let at = hash(&key) as usize % 193;
        let e = Entry {
            key,
            references: 1,
            resource: 7,
        };
        let mut b = vec![vec![]; 193];
        b[(at + 1) % 193].push(e.clone());
        assert!(Cache::from_retained(b, 1).is_err());
        let mut b = vec![vec![]; 193];
        b[at] = vec![e.clone(), e];
        assert!(Cache::from_retained(b, 2).is_err());
    }
}
