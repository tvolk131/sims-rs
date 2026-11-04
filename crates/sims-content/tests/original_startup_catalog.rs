use sha2::{Digest, Sha256};
use sims_content::{Index, object_catalog::Catalog};
use std::{path::PathBuf, sync::Arc};

#[test]
#[ignore = "requires the preserved private Wine Newbie installation"]
fn registered_catalog_and_factory_inputs_match_original_order() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache/wine-parity-import-1/assets");
    let index = Arc::new(Index::scan(&root).unwrap());
    let mut catalog = Catalog::household(
        index.clone(),
        "UserData/Neighborhood.iff",
        Default::default(),
    )
    .unwrap();
    // Captured original catalog: each row is GUID, kind, global flag, OBJD id
    // in little endian. This tests all registrations, not only the two Sims.
    let mut all = Vec::new();
    let mut expected_controllers = vec![];
    for e in &catalog.definitions.entries {
        let guid = u32::from_le_bytes(e.data[28..32].try_into().unwrap());
        let kind = u16::from_le_bytes(e.data[18..20].try_into().unwrap());
        let global = u16::from_le_bytes(e.data[84..86].try_into().unwrap());
        all.extend_from_slice(&guid.to_le_bytes());
        all.extend_from_slice(&kind.to_le_bytes());
        all.extend_from_slice(&global.to_le_bytes());
        all.extend_from_slice(&e.key.chunk.to_le_bytes());
        if matches!(kind, 7 | 12) && global != 0 {
            expected_controllers.push((guid, e.key.chunk));
        }
    }
    assert_eq!(catalog.definitions.entries.len(), 5454);
    assert_eq!(
        format!("{:x}", Sha256::digest(all)),
        "e957604d182678a9d6a38d28bc543589a6341d9ae273f49cf952c273e26367ea"
    );
    let controllers = catalog.global_controllers(&index).unwrap();
    assert_eq!(controllers.len(), 141);
    // Captured process-owned attribute directory at the actual resize entry.
    let storage = catalog.resource_storage(&index).unwrap();
    assert_eq!(
        storage
            .iter()
            .map(|d| (d.guid, d.count))
            .collect::<Vec<_>>(),
        [
            (0x8963d7c1, 24),
            (0x22eae6b4, 1),
            (0xd557147b, 32),
            (0xf9edb066, 32),
            (0x69347d77, 32),
            (0x96149594, 32),
            (0x48eead1c, 32),
            (0x9afbbcb0, 1),
            (0x1fac2240, 12),
            (0x552a8297, 10),
            (0xfcf70513, 9),
        ]
    );
    assert_eq!(
        sims_vm::resource_storage::State::construct(&storage)
            .records
            .iter()
            .map(|r| (r.guid, &r.values))
            .collect::<Vec<_>>(),
        catalog
            .definitions
            .attributes
            .iter()
            .map(|r| (r.guid, &r.values))
            .collect::<Vec<_>>()
    );
    assert!(storage.iter().all(|d| !d.reset_per_lot));
    assert_eq!(
        controllers
            .iter()
            .map(|c| (c.guid, c.definition.chunk))
            .collect::<Vec<_>>(),
        expected_controllers
    );
    for controller in controllers {
        let bytes = index.resources[index.find(&controller.definition.source).unwrap()]
            .read()
            .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            controller.definition.sha256
        );
    }
}
