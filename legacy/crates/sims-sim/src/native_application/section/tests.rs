use super::*;

fn request() -> SectionRequest {
    SectionRequest {
        instruction: 0x5d91bc,
        import_slot: 0x666114,
        region: Owner {
            token: 0x10000018,
            data: vec![0x72; 24],
        },
    }
}

#[test]
fn full_layout_initializes_every_byte_and_retains_the_actual_debug_identity() {
    let request = request();
    let input = Owner {
        token: 0x40000000,
        data: (0..32).map(|i| i + 0x81).collect(),
    };
    let original = input.clone();
    let result = InitializedSection::from_debug_info(&request, Some(&input), 9).unwrap();
    assert_eq!(input, original);
    assert_eq!(request.region.data, vec![0x72; 24]);
    assert_eq!(
        result.region.data,
        words(&[0x40000000, u32::MAX, 0, 0, 0, 0])
    );
    assert_eq!(
        result.debug_info.as_ref().unwrap().data,
        words(&[0, 0x10000018, 0x40000008, 0x40000008, 0, 0, 0, 0])
    );
    result.validate(&request).unwrap();
    println!(
        "RTL_SECTION_IMAGES {}",
        serde_json::json!({"section":result.region.data,"debug":result.debug_info.as_ref().unwrap().data})
    );
}

#[test]
fn actual_nullable_debug_allocation_branch_still_initializes_the_section() {
    let request = request();
    let result = InitializedSection::from_debug_info(&request, None, 9).unwrap();
    assert_eq!(result.region.data, words(&[0, u32::MAX, 0, 0, 0, 0]));
    assert!(result.debug_info.is_none());
    result.validate(&request).unwrap();
}

#[test]
fn malformed_debug_extents_pointers_and_aliases_fail_before_losing_inputs() {
    let request = request();
    for input in [
        Owner {
            token: request.region.token,
            data: vec![0x73; 32],
        },
        Owner {
            token: 0x40000000,
            data: vec![0x73; 31],
        },
        Owner {
            token: u64::from(u32::MAX) - 8,
            data: vec![0x73; 32],
        },
    ] {
        let original = input.clone();
        assert!(InitializedSection::from_debug_info(&request, Some(&input), 9).is_err());
        assert_eq!(input, original);
    }
}

#[test]
fn returned_graph_rejects_lost_owner_backlink_and_mutated_header() {
    let request = request();
    let input = Owner {
        token: 0x40000000,
        data: vec![0; 32],
    };
    for change in 0..4 {
        let mut result = InitializedSection::from_debug_info(&request, Some(&input), 9).unwrap();
        match change {
            0 => result.debug_info = None,
            1 => result.debug_info.as_mut().unwrap().data[4] ^= 1,
            2 => result.region.data[4] = 0,
            _ => result.capability = 0,
        }
        assert!(result.validate(&request).is_err());
    }
}
