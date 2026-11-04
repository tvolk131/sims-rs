use sha2::{Digest, Sha256};
use sims_content::{Index, Resource, saves::definitions};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

fn object_definition(guid: u32, kind: u16, multipart: u16) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[18..20].copy_from_slice(&kind.to_le_bytes());
    bytes[20..22].copy_from_slice(&multipart.to_le_bytes());
    bytes[28..32].copy_from_slice(&guid.to_le_bytes());
    bytes
}

fn resource(locator: &str, chunks: &[(u16, Vec<u8>)]) -> Resource {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (chunk, payload) in chunks {
        bytes.extend_from_slice(b"OBJD");
        bytes.extend_from_slice(&(76 + payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&chunk.to_be_bytes());
        bytes.extend_from_slice(&[0; 66]);
        bytes.extend_from_slice(payload);
    }
    Resource {
        locator: locator.into(),
        name: locator.rsplit('/').next().unwrap().into(),
        source: PathBuf::from("unopened-owned-resource"),
        offset: 0,
        size: bytes.len() as u64,
        unpacked_size: bytes.len() as u64,
        archived: false,
        owned: Some(Arc::from(bytes)),
    }
}

fn index(resources: Vec<Resource>) -> Index {
    Index {
        root: PathBuf::new(),
        resources,
        archives: 0,
        diagnostics: Vec::new(),
    }
}

#[test]
fn multiple_matching_chunks_keep_whole_file_hash_and_definition_metadata() {
    let selected = resource(
        "Objects/shared.iff",
        &[
            (17, object_definition(0x1234, 2, 0)),
            (33, object_definition(0x5678, 4, 2)),
            (34, vec![0xff; 31]),
            (35, object_definition(0x9999, 7, 1)),
        ],
    );
    let original = selected.read().unwrap();
    let expected_hash = format!("{:x}", Sha256::digest(&original));
    let resources = index(vec![selected, resource("Objects/empty.iff", &[])]);
    let resolved = definitions(&resources, "UserData", &BTreeSet::from([0x1234, 0x5678])).unwrap();
    assert_eq!(resolved.len(), 2);
    for (guid, chunk, kind, multipart) in [(0x1234, 17, 2, false), (0x5678, 33, 4, true)] {
        let selected = &resolved[&guid];
        assert_eq!(selected.source, "Objects/shared.iff");
        assert_eq!(selected.sha256, expected_hash);
        assert_eq!(
            (selected.chunk, selected.kind, selected.multipart),
            (chunk, kind, multipart)
        );
    }
    assert_eq!(resources.resources[0].read().unwrap(), original);
}

#[test]
fn identical_providers_keep_original_resource_order() {
    let chunks = [
        (17, object_definition(0x1234, 2, 0)),
        (33, object_definition(0x5678, 4, 1)),
    ];
    let resources = index(vec![
        resource("Objects/z-first.iff", &chunks),
        resource("Objects/a-second.iff", &chunks),
    ]);
    let resolved = definitions(&resources, "UserData", &BTreeSet::from([0x1234, 0x5678])).unwrap();
    assert!(
        resolved
            .values()
            .all(|selected| selected.source == "Objects/z-first.iff")
    );
}

#[test]
fn conflicting_provider_bytes_or_chunk_ids_remain_errors() {
    for alternative in [
        resource(
            "Objects/second.iff",
            &[(17, object_definition(0x1234, 3, 0))],
        ),
        resource(
            "Objects/second.iff",
            &[(18, object_definition(0x1234, 2, 0))],
        ),
    ] {
        let resources = index(vec![
            resource(
                "Objects/first.iff",
                &[(17, object_definition(0x1234, 2, 0))],
            ),
            alternative,
        ]);
        let error = definitions(&resources, "UserData", &BTreeSet::from([0x1234])).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("ambiguous original object 00001234:")
        );
    }
    let resources = index(vec![resource(
        "Objects/shared.iff",
        &[
            (17, object_definition(0x1234, 2, 0)),
            (18, object_definition(0x1234, 2, 0)),
        ],
    )]);
    assert!(
        definitions(&resources, "UserData", &BTreeSet::from([0x1234]))
            .unwrap_err()
            .to_string()
            .starts_with("ambiguous original object 00001234:")
    );
}

#[test]
fn local_character_precedence_and_foreign_exclusion_remain_scoped() {
    let mut foreign = resource("UserData2/Characters/other.iff", &[]);
    foreign.owned = Some(Arc::from(vec![0; 64]));
    let resources = index(vec![
        resource(
            "Objects/global.iff",
            &[(17, object_definition(0x1234, 2, 0))],
        ),
        foreign,
        resource(
            "UserData/Characters/local.iff",
            &[(33, object_definition(0x1234, 4, 1))],
        ),
    ]);
    let resolved = definitions(&resources, "UserData", &BTreeSet::from([0x1234])).unwrap();
    assert_eq!(resolved[&0x1234].source, "UserData/Characters/local.iff");
    assert_eq!((resolved[&0x1234].chunk, resolved[&0x1234].kind), (33, 4));
}

#[test]
fn short_and_nonmatching_chunks_preserve_missing_guid_errors() {
    let resources = index(vec![resource(
        "Objects/other.iff",
        &[(17, vec![0xff; 31]), (33, object_definition(0x5678, 4, 1))],
    )]);
    let error = definitions(&resources, "UserData", &BTreeSet::from([0x1234])).unwrap_err();
    assert_eq!(error.to_string(), "missing original object GUID 00001234");
    assert!(
        definitions(&resources, "UserData", &BTreeSet::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn eligible_malformed_resources_are_still_parsed_after_a_match_or_with_no_guids() {
    let mut malformed = resource("Objects/malformed.iff", &[]);
    malformed.owned = Some(Arc::from(vec![0; 64]));
    let resources = index(vec![
        resource(
            "Objects/first.iff",
            &[(17, object_definition(0x1234, 2, 0))],
        ),
        malformed,
    ]);
    for guids in [BTreeSet::from([0x1234]), BTreeSet::new()] {
        let error = definitions(&resources, "UserData", &guids).unwrap_err();
        assert!(error.to_string().contains("invalid Maxis IFF signature"));
    }
}

#[test]
fn digest_reuse_does_not_survive_replaced_owned_bytes() {
    let mut resources = index(vec![resource(
        "Objects/shared.iff",
        &[
            (17, object_definition(0x1234, 2, 0)),
            (33, object_definition(0x5678, 4, 1)),
        ],
    )]);
    let guids = BTreeSet::from([0x1234, 0x5678]);
    let before = definitions(&resources, "UserData", &guids).unwrap();
    resources.resources[0] = resource(
        "Objects/shared.iff",
        &[
            (17, object_definition(0x1234, 6, 0)),
            (33, object_definition(0x5678, 4, 1)),
        ],
    );
    let expected_hash = format!(
        "{:x}",
        Sha256::digest(resources.resources[0].read().unwrap())
    );
    let after = definitions(&resources, "UserData", &guids).unwrap();
    for guid in &guids {
        assert_ne!(after[guid].sha256, before[guid].sha256);
        assert_eq!(after[guid].sha256, expected_hash);
    }
    assert_eq!(after[&0x1234].kind, 6);
    assert_eq!(after[&0x5678].kind, before[&0x5678].kind);
}
