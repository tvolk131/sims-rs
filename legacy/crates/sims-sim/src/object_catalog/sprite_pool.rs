//! 4c6f01/4c6f95 borrowed sprite-resource wrappers. Their references are
//! independent of object-resource references; releasing the last wrapper does
//! not release/destroy its borrowed underlying resource. Populated sprite and
//! renderer caches are owned by a separate presentation adapter.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub wrapper: u32,
    pub resource: u32,
    pub references: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pool {
    pub entries: Vec<Entry>,
    next: u32,
}

/// A newly declared adapter identity domain. Native heap allocation does not
/// supply the monotonic counter used by this Rust owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum IdentityEpoch {
    NewUnpublished { next: u32 },
}
impl Default for Pool {
    fn default() -> Self {
        Self {
            entries: vec![],
            next: 1,
        }
    }
}
impl Pool {
    /// Retain wrapper aliases/order/references without executing acquire.
    pub fn from_retained(entries: Vec<Entry>, epoch: IdentityEpoch) -> Result<Self, String> {
        let IdentityEpoch::NewUnpublished { next } = epoch;
        let mut wrappers = std::collections::BTreeSet::new();
        let mut resources = std::collections::BTreeSet::new();
        if next == 0
            || entries.iter().any(|e| {
                e.wrapper == 0
                    || e.wrapper >= next
                    || !wrappers.insert(e.wrapper)
                    || !resources.insert(e.resource)
            })
        {
            return Err("invalid retained sprite wrapper identity owner".into());
        }
        Ok(Self { entries, next })
    }

    pub fn acquire(&mut self, resource: u32) -> Result<u32, String> {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.resource == resource) {
            entry.references = entry.references.wrapping_add(1);
            return Ok(entry.wrapper);
        }
        let wrapper = self.next;
        self.next = wrapper
            .checked_add(1)
            .ok_or("sprite wrapper identity exhausted")?;
        self.entries.push(Entry {
            wrapper,
            resource,
            references: 1,
        });
        Ok(wrapper)
    }
    /// True means this wrapper was destroyed. Unknown wrappers do nothing;
    /// reference arithmetic wraps exactly like the original 32-bit word.
    pub fn release(&mut self, wrapper: u32) -> bool {
        let Some(index) = self.entries.iter().position(|e| e.wrapper == wrapper) else {
            return false;
        };
        let entry = &mut self.entries[index];
        entry.references = entry.references.wrapping_sub(1);
        if entry.references != 0 {
            return false;
        }
        self.entries.remove(index);
        true
    }
}

#[cfg(test)]
mod retained_tests {
    use super::*;

    #[test]
    fn import_keeps_order_aliases_and_references() {
        let entries = vec![
            Entry {
                wrapper: 8,
                resource: 2,
                references: 0,
            },
            Entry {
                wrapper: 3,
                resource: 7,
                references: u32::MAX,
            },
        ];
        let mut pool =
            Pool::from_retained(entries.clone(), IdentityEpoch::NewUnpublished { next: 9 })
                .unwrap();
        assert_eq!(pool.entries, entries);
        assert_eq!(pool.acquire(19).unwrap(), 9);
    }

    #[test]
    fn aliases_and_missing_identity_counter_are_rejected() {
        let e = Entry {
            wrapper: 3,
            resource: 7,
            references: 1,
        };
        assert!(
            Pool::from_retained(vec![e.clone()], IdentityEpoch::NewUnpublished { next: 3 })
                .is_err()
        );
        assert!(
            Pool::from_retained(
                vec![e.clone(), e.clone()],
                IdentityEpoch::NewUnpublished { next: 4 }
            )
            .is_err()
        );
        assert!(
            Pool::from_retained(
                vec![e.clone(), Entry { wrapper: 2, ..e }],
                IdentityEpoch::NewUnpublished { next: 4 }
            )
            .is_err()
        );
        assert!(serde_json::from_str::<IdentityEpoch>("{}").is_err());
        assert!(serde_json::from_str::<IdentityEpoch>(r#"{"NewUnpublished":{}}"#).is_err());
        assert!(
            serde_json::from_str::<IdentityEpoch>(r#"{"RecoveredNative":{"next":4}}"#).is_err()
        );
    }
}
