//! Import the already open readonly Global provider, without executing Open or
//! rebuilding its hash tables. Native pointer identities are translated by the
//! caller; the evidence for that translation stays outside the gameplay owner.
use super::*;

/// All mutable modeled inputs are required. In particular, an empty resident
/// map or a fresh stream cursor is not a substitute for an unavailable owner.
///
/// Handles are stable adapter identities, not native pointers. `next_handle`
/// is the adapter's next unused identity; native allocation has no equivalent
/// monotonic counter. `buffers` owns any already detached adapter allocations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retained {
    pub aggregate: Composite,
    pub iff: Iff,
    pub stream_position: u32,
    pub stream_open: bool,
    pub stream_writable: bool,
    pub handles: BTreeMap<u32, u32>,
    pub next_handle: u32,
    pub buffers: BTreeMap<u32, Vec<u8>>,
}

impl Provider {
    /// Consume observed state for the pinned readonly Global.iff source. This
    /// validates ownership but does not certify the caller's capture provenance.
    /// No lookup, payload conversion, Open, seek, or diagnostic event is issued.
    ///
    /// The original substream's backend file handle and read cache are platform
    /// services: this owner uses the hash-checked immutable module bytes. The
    /// caller must establish the same readonly logical stream and relative
    /// cursor; pending writes or a writable source are outside this profile.
    pub fn from_retained(index: &Index, locator: &str, retained: Retained) -> Result<Self> {
        let source = Source::observed(locator);
        let bytes = source.bytes(index)?;
        let provider = Self::retain_bytes(source, bytes, retained);
        provider.validate(index)?;
        Ok(provider)
    }

    fn retain_bytes(source: Source, bytes: Vec<u8>, retained: Retained) -> Self {
        let Retained {
            aggregate,
            iff,
            stream_position,
            stream_open,
            stream_writable,
            handles,
            next_handle,
            buffers,
        } = retained;
        Self {
            source,
            aggregate,
            components: Components {
                entries: BTreeMap::from([(
                    1,
                    Component {
                        iff,
                        stream: OwnedStream {
                            bytes,
                            position: stream_position,
                            open: stream_open,
                            writable: stream_writable,
                        },
                    },
                )]),
                handles: handles
                    .into_iter()
                    .map(|(h, local)| (h, (1, local)))
                    .collect(),
                next_handle,
                buffers,
            },
            events: vec![],
        }
    }
}

#[cfg(test)]
mod captured;

#[cfg(test)]
mod tests {
    use super::*;

    fn retained(components: Components) -> (Vec<u8>, Retained) {
        let Component { iff, stream } = components.entries[&1].clone();
        (
            stream.bytes,
            Retained {
                aggregate: super::super::tests::aggregate(),
                iff,
                stream_position: stream.position,
                stream_open: stream.open,
                stream_writable: stream.writable,
                handles: components
                    .handles
                    .into_iter()
                    .map(|(h, (_, l))| (h, l))
                    .collect(),
                next_handle: components.next_handle,
                buffers: components.buffers,
            },
        )
    }

    #[test]
    fn import_keeps_hot_payload_flags_topology_and_cursor_without_services() {
        let mut components = super::super::tests::components(true);
        let mut aggregate = super::super::tests::aggregate();
        let handle = aggregate
            .by_id(TAG, 2, 0, CALLBACK, &mut components)
            .unwrap();
        let local = components.handles[&handle].1;
        // Retained mutable payloads must not be reconstructed from disk.
        components
            .entries
            .get_mut(&1)
            .unwrap()
            .iff
            .payload_mut(local)
            .unwrap()[0] ^= 1;
        let expected = components.clone();
        let (bytes, mut input) = retained(components);
        input.stream_position = 129;
        let expected_iff = input.iff.clone();
        let input: Retained = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        let mut imported = Provider::retain_bytes(Source::observed("fixture"), bytes, input);
        imported.components.validate().unwrap();
        assert_eq!(imported.components.entries[&1].iff, expected_iff);
        assert_eq!(imported.components.handles, expected.handles);
        assert_eq!(imported.components.next_handle, expected.next_handle);
        assert!(imported.events.is_empty());
        assert_eq!(imported.components.entries[&1].stream.position, 129);
        // A resident lookup returns the same allocation without touching I/O.
        assert_eq!(
            imported
                .aggregate
                .by_id(TAG, 2, 0, CALLBACK, &mut imported.components)
                .unwrap(),
            handle
        );
        assert_eq!(imported.components.entries[&1].stream.position, 129);
        assert_eq!(imported.components.entries[&1].iff, expected_iff);
    }

    #[test]
    fn import_preserves_detached_allocations_and_rejects_lost_or_aliased_owners() {
        let mut c = super::super::tests::components(true);
        let mut a = super::super::tests::aggregate();
        let h = a.by_id(TAG, 2, 0, CALLBACK, &mut c).unwrap();
        a.release_chunk(h, &mut c).unwrap();
        let expected = c.clone();
        let (bytes, input) = retained(c);
        let imported =
            Provider::retain_bytes(Source::observed("fixture"), bytes.clone(), input.clone());
        imported.components.validate().unwrap();
        assert_eq!(imported.components, expected);
        let mut lost = input.clone();
        lost.buffers.clear();
        assert!(
            Provider::retain_bytes(Source::observed("fixture"), bytes.clone(), lost)
                .components
                .validate()
                .is_err()
        );
        let mut alias = input;
        alias.handles.insert(alias.next_handle, alias.handles[&h]);
        alias.next_handle += 1;
        assert!(
            Provider::retain_bytes(Source::observed("fixture"), bytes, alias)
                .components
                .validate()
                .is_err()
        );
    }

    #[test]
    fn no_required_import_state_can_default_or_be_misspelled() {
        let (_, retained) = retained(super::super::tests::components(true));
        let value = serde_json::to_value(retained).unwrap();
        for field in value.as_object().unwrap().keys() {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<Retained>(missing).is_err(),
                "{field}"
            );
        }
        let mut unknown = value;
        unknown["stream_postion"] = 0.into();
        assert!(serde_json::from_value::<Retained>(unknown).is_err());
    }

    #[test]
    fn invalid_record_index_is_rejected_before_resident_handle_lookup() {
        let mut c = super::super::tests::components(true);
        let mut a = super::super::tests::aggregate();
        a.by_id(TAG, 2, 0, CALLBACK, &mut c).unwrap();
        let (bytes, input) = retained(c);
        let mut value = serde_json::to_value(input).unwrap();
        let outer = value["iff"]["table"]["buckets"].as_array_mut().unwrap();
        let tag = outer
            .iter_mut()
            .find(|b| !b.as_array().unwrap().is_empty())
            .unwrap();
        let inner = tag[0][1]["buckets"].as_array_mut().unwrap();
        let record = inner
            .iter_mut()
            .find(|b| !b.as_array().unwrap().is_empty())
            .unwrap();
        record[0][1] = 999999.into();
        let input: Retained = serde_json::from_value(value).unwrap();
        let bad = Provider::retain_bytes(Source::observed("fixture"), bytes, input);
        assert!(bad.components.validate().is_err());
    }
}
