use super::*;
use sims_formats::{
    animation::{Animation, Motion, TimedProperty},
    avatar::{Appearance, Binding, Bone, Skeleton, Wiggle},
};

#[derive(Clone, Default)]
struct OriginalPool {
    next: AssetId,
    values: BTreeMap<AssetId, Loaded>,
}

impl OriginalPool {
    fn insert(&mut self, value: Loaded) -> Result<AssetId, String> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or("avatar asset identity exhausted")?;
        self.values.insert(self.next, value);
        Ok(self.next)
    }
}

impl Assets for OriginalPool {
    fn name(&self, asset: AssetId) -> Result<Vec<u8>, String> {
        let name = self
            .values
            .get(&asset)
            .ok_or("decoded avatar asset absent")?
            .value
            .name();
        let (bytes, _, invalid) = encoding_rs::WINDOWS_1252.encode(name);
        if invalid {
            return Err("avatar name is not Windows-1252".into());
        }
        Ok(bytes.into_owned())
    }
    fn rename(&mut self, asset: AssetId, name: Vec<u8>) -> Result<(), String> {
        self.values
            .get_mut(&asset)
            .ok_or("decoded avatar asset absent")?
            .value
            .set_name(encoding_rs::WINDOWS_1252.decode(&name).0.into_owned());
        Ok(())
    }
    fn dispose(&mut self, asset: AssetId) -> Result<(), String> {
        self.values
            .remove(&asset)
            .ok_or("disposed avatar asset absent")?;
        Ok(())
    }
}

fn loaded(name: &str, source: usize) -> Loaded {
    Loaded {
        source,
        context: source as u32 + 100,
        value: Asset::Appearance(Appearance {
            name: name.into(),
            kind: 7,
            bindings: vec![Binding {
                bone: "spine".into(),
                mesh: "mesh".into(),
                censor: 3,
            }],
        }),
    }
}

fn equivalent(pool: &Pool, original: &OriginalPool) {
    assert_eq!(pool.next, original.next);
    assert_eq!(
        format!("{:?}", pool.values),
        format!("{:?}", original.values)
    );
    for identity in pool.values.keys() {
        assert_eq!(pool.name(*identity), original.name(*identity));
    }
}

fn pools() -> (Pool, OriginalPool) {
    let mut pool = Pool::default();
    let mut original = OriginalPool::default();
    for (source, name) in ["First", "Second", "\u{4e2d}"].into_iter().enumerate() {
        assert_eq!(
            pool.insert(loaded(name, source)),
            original.insert(loaded(name, source))
        );
    }
    equivalent(&pool, &original);
    (pool, original)
}

#[test]
fn shared_pool_payloads_detach_for_mutation_without_changing_identity_or_errors() {
    let (mut source, mut expected_source) = pools();
    let mut branch = source.clone();
    let mut expected_branch = expected_source.clone();
    let mut sibling = source.clone();
    let mut expected_sibling = expected_source.clone();
    for identity in source.values.keys() {
        assert!(Arc::ptr_eq(
            &source.values[identity],
            &branch.values[identity]
        ));
        assert!(Arc::ptr_eq(
            &source.values[identity],
            &sibling.values[identity]
        ));
    }
    let shared = source.values[&1].clone();
    assert_eq!(
        branch.rename(99, b"missing".to_vec()),
        expected_branch.rename(99, b"missing".to_vec())
    );
    assert!(Arc::ptr_eq(&shared, &branch.values[&1]));
    for name in [b"MiXeD".to_vec(), vec![0xe9, 0x81, 0], b"last".to_vec()] {
        assert_eq!(
            branch.rename(1, name.clone()),
            expected_branch.rename(1, name)
        );
        equivalent(&branch, &expected_branch);
        equivalent(&source, &expected_source);
        equivalent(&sibling, &expected_sibling);
    }
    assert!(!Arc::ptr_eq(&source.values[&1], &branch.values[&1]));
    assert!(Arc::ptr_eq(&source.values[&2], &branch.values[&2]));
    assert_eq!(
        source.rename(2, b"source-only".to_vec()),
        expected_source.rename(2, b"source-only".to_vec())
    );
    assert!(!Arc::ptr_eq(&source.values[&2], &branch.values[&2]));
    for _ in 0..2 {
        assert_eq!(branch.dispose(2), expected_branch.dispose(2));
        equivalent(&branch, &expected_branch);
    }
    assert_eq!(
        sibling.insert(loaded("new sibling", 22)),
        expected_sibling.insert(loaded("new sibling", 22))
    );
    assert_eq!(
        branch.insert(loaded("new branch", 33)),
        expected_branch.insert(loaded("new branch", 33))
    );
    equivalent(&source, &expected_source);
    equivalent(&sibling, &expected_sibling);
    equivalent(&branch, &expected_branch);
    assert!(!Arc::ptr_eq(&branch.values[&4], &sibling.values[&4]));
    branch.next = AssetId::MAX;
    expected_branch.next = AssetId::MAX;
    assert_eq!(
        branch.insert(loaded("overflow", 44)),
        expected_branch.insert(loaded("overflow", 44))
    );
    equivalent(&branch, &expected_branch);
}

#[test]
fn native_record_replacement_preserves_partial_failures_and_other_pool_owners() {
    for incoming in [2, 99] {
        let (source, expected_source) = pools();
        let mut branch = source.clone();
        let mut expected_branch = expected_source.clone();
        let mut record = Record {
            location: catalog_index::Lazy {
                name: b"MiXeD\0tail".to_vec(),
                directory: b"directory".to_vec(),
                catalog: b"catalog".to_vec(),
                context: 77,
            },
            attempted: false,
            asset: Some(1),
        };
        let mut expected_record = record.clone();
        assert_eq!(
            record.replace(Kind::Appearance, incoming, &mut branch),
            expected_record.replace(Kind::Appearance, incoming, &mut expected_branch)
        );
        assert_eq!(record, expected_record);
        equivalent(&branch, &expected_branch);
        equivalent(&source, &expected_source);
        assert!(!branch.values.contains_key(&1));
        assert!(source.values.contains_key(&1));
        if incoming == 2 {
            assert_eq!(branch.name(2).unwrap(), b"mixed");
            assert_eq!(source.name(2).unwrap(), b"Second");
            assert!(!Arc::ptr_eq(&source.values[&2], &branch.values[&2]));
        } else {
            assert_eq!(record.asset, Some(99));
            assert!(!record.attempted);
            assert!(Arc::ptr_eq(&source.values[&2], &branch.values[&2]));
        }
    }
}

#[test]
fn catalog_clones_preserve_public_asset_views_and_private_registry_isolation() {
    let mut source = Catalog::default();
    let asset = source.assets.insert(loaded("Ready", 7)).unwrap();
    source
        .registry
        .register_loaded(
            Kind::Appearance,
            catalog_index::Lazy {
                name: vec![],
                directory: b"directory".to_vec(),
                catalog: b"catalog".to_vec(),
                context: 91,
            },
            asset,
            &mut source.assets,
        )
        .unwrap();
    let mut branch = source.clone();
    assert!(Arc::ptr_eq(
        &source.assets.values[&asset],
        &branch.assets.values[&asset]
    ));
    let selected: &Loaded = branch.asset(Kind::Appearance, b"Ready").unwrap();
    assert_eq!(selected.source, 7);
    assert_eq!(selected.context, 107);
    assert_eq!(selected.value.name(), "Ready");
    branch
        .assets
        .rename(asset, b"branch-only".to_vec())
        .unwrap();
    assert_eq!(
        source
            .asset(Kind::Appearance, b"Ready")
            .unwrap()
            .value
            .name(),
        "Ready"
    );
    assert_eq!(
        branch
            .asset(Kind::Appearance, b"Ready")
            .unwrap()
            .value
            .name(),
        "branch-only"
    );
    branch.assets.dispose(asset).unwrap();
    assert!(branch.asset(Kind::Appearance, b"Ready").is_none());
    assert!(source.asset(Kind::Appearance, b"Ready").is_some());
    branch.fault = Some("branch-only fault".into());
    assert!(branch.ready().is_err());
    source.ready().unwrap();
    assert_eq!(
        source.registry.get(Kind::Appearance, b"Ready"),
        branch.registry.get(Kind::Appearance, b"Ready")
    );
}

#[test]
fn nested_payload_clones_detach_and_release_only_after_the_last_pool_owner() {
    let event = TimedProperty {
        time_ms: -7,
        bone: "spine".into(),
        pairs: vec![("event".into(), "original".into())],
    };
    let values = [
        Asset::Skeleton(Skeleton {
            name: "Skeleton".into(),
            bones: vec![Bone {
                name: "spine".into(),
                parent: None,
                translation: [1.0, 2.0, 3.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                properties: vec![vec![("property".into(), "original".into())]],
                flags: [1, 0, 1],
                wiggle: Wiggle {
                    gradient_scale: 17,
                    amplitude: 0.25,
                },
            }],
        }),
        Asset::Animation(Animation {
            name: "Animation".into(),
            skill: "skill".into(),
            duration_ms: 120.0,
            distance: 2.0,
            moving: true,
            translation_count: 1,
            rotation_count: 1,
            frames: 2,
            motions: vec![Motion {
                bone: "spine".into(),
                frames: 2,
                duration_ms: 120.0,
                translation: Some(0),
                rotation: Some(0),
                properties: vec![vec![("property".into(), "original".into())]],
                event_lists: vec![vec![event.clone()]],
            }],
            events: vec![event],
        }),
    ];
    for value in values {
        let mut source = Pool::default();
        let mut expected = OriginalPool::default();
        let loaded = Loaded {
            source: 17,
            context: 91,
            value,
        };
        let identity = source.insert(loaded.clone()).unwrap();
        assert_eq!(expected.insert(loaded).unwrap(), identity);
        let mut branch = source.clone();
        let mut expected_branch = expected.clone();
        let mut sibling = source.clone();
        let weak = Arc::downgrade(&source.values[&identity]);
        assert_eq!(weak.strong_count(), 3);
        let mut independent = source.values[&identity].as_ref().clone();
        match &mut independent.value {
            Asset::Skeleton(skeleton) => {
                skeleton.bones[0].properties[0][0]
                    .1
                    .push_str("-independent");
                skeleton.bones[0].wiggle.gradient_scale = 99;
            }
            Asset::Animation(animation) => {
                animation.motions[0].event_lists[0][0].pairs[0]
                    .1
                    .push_str("-independent");
                animation.events[0].pairs[0].1.push_str("-independent");
            }
            Asset::Appearance(_) => unreachable!(),
        }
        assert_ne!(
            format!("{:?}", independent),
            format!("{:?}", source.values[&identity])
        );
        equivalent(&source, &expected);
        equivalent(&sibling, &expected);
        assert_eq!(
            branch.rename(identity, b"branch-only".to_vec()),
            expected_branch.rename(identity, b"branch-only".to_vec())
        );
        equivalent(&branch, &expected_branch);
        equivalent(&source, &expected);
        assert!(!Arc::ptr_eq(
            &source.values[&identity],
            &branch.values[&identity]
        ));
        assert_eq!(weak.strong_count(), 2);
        source.dispose(identity).unwrap();
        assert_eq!(weak.strong_count(), 1);
        assert!(weak.upgrade().is_some());
        sibling.dispose(identity).unwrap();
        assert!(weak.upgrade().is_none());
        equivalent(&branch, &expected_branch);
    }
}

#[test]
fn replacement_missing_previous_and_same_identity_match_the_original_pool() {
    for (previous, incoming) in [(99, 2), (1, 1)] {
        let (source, expected_source) = pools();
        let mut branch = source.clone();
        let mut expected_branch = expected_source.clone();
        let mut record = Record {
            location: catalog_index::Lazy {
                name: b"Replacement".to_vec(),
                directory: b"directory".to_vec(),
                catalog: b"catalog".to_vec(),
                context: 17,
            },
            attempted: false,
            asset: Some(previous),
        };
        let mut expected_record = record.clone();
        let actual = record.replace(Kind::Appearance, incoming, &mut branch);
        assert!(actual.is_err());
        assert_eq!(
            actual,
            expected_record.replace(Kind::Appearance, incoming, &mut expected_branch)
        );
        assert_eq!(record, expected_record);
        equivalent(&branch, &expected_branch);
        equivalent(&source, &expected_source);
    }
}

#[test]
fn cloned_catalog_registration_lazy_attempts_and_failure_lists_are_independent() {
    let mut source = Catalog::default();
    let location = catalog_index::Lazy {
        name: b"Pending".to_vec(),
        directory: b"directory/".to_vec(),
        catalog: b"catalog".to_vec(),
        context: 17,
    };
    assert!(
        source
            .registry
            .insert(Kind::Appearance, Record::lazy(location.clone()))
    );
    let mut missing = location.clone();
    missing.name = b"Missing".to_vec();
    assert!(
        source
            .registry
            .insert(Kind::Appearance, Record::lazy(missing))
    );
    let before = format!("{source:?}");
    let mut branch = source.clone();
    for (name, expected) in [
        ("Branch", "Inserted"),
        ("Pending", "Replaced"),
        ("Pending", "Discarded"),
    ] {
        let incoming = branch.assets.insert(loaded(name, 7)).unwrap();
        let result = branch
            .registry
            .register_loaded(
                Kind::Appearance,
                location.clone(),
                incoming,
                &mut branch.assets,
            )
            .unwrap();
        assert_eq!(format!("{result:?}"), expected);
        assert_eq!(
            branch.assets.values.contains_key(&incoming),
            expected != "Discarded"
        );
        assert_eq!(format!("{source:?}"), before);
    }
    assert!(branch.asset(Kind::Appearance, b"Branch").is_some());
    assert_eq!(
        branch
            .asset(Kind::Appearance, b"Pending")
            .unwrap()
            .value
            .name(),
        "pending"
    );
    let index = Index {
        root: std::path::PathBuf::new(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    };
    assert!(
        branch
            .resolve(
                &index,
                &Directories::default(),
                Kind::Appearance,
                b"Missing"
            )
            .unwrap()
            .is_none()
    );
    assert!(branch.get(Kind::Appearance, b"Missing").unwrap().attempted);
    assert!(!source.get(Kind::Appearance, b"Missing").unwrap().attempted);
    branch.read_failures.push(7);
    branch.index_read_failures.push(8);
    assert!(
        branch
            .completed::<()>(Err("branch-only fault".into()))
            .is_err()
    );
    assert!(branch.ready().is_err());
    source.ready().unwrap();
    assert_eq!(format!("{source:?}"), before);
}
