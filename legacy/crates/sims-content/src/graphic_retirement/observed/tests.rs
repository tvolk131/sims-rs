use super::*;

fn authored() -> Retained {
    serde_json::from_str(include_str!("authored.json")).unwrap()
}
fn cold<T: Serialize + serde::de::DeserializeOwned>(x: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(x).unwrap()).unwrap()
}

#[test]
fn every_service_cold_cut_keeps_surviving_skin_and_exact_lifetimes() {
    for references in [0, 2, 15, u32::MAX] {
        let mut input = authored();
        input.skin.references = references;
        let before = input.clone();
        let mut warm = Owner::from_retained(input).unwrap();
        let mut cold_owner = cold(&warm);
        let mut services = 0;
        while !warm.complete() {
            assert_eq!(warm.advance().unwrap(), cold_owner.advance().unwrap());
            services += 1;
            assert!(services < 32);
            cold_owner = cold(&cold_owner);
            cold_owner.validate().unwrap();
            assert_eq!(warm, cold_owner);
        }
        assert_eq!(warm.retained.skin.references, references.wrapping_sub(1));
        assert_eq!(
            warm.retained.skin.generation,
            before.skin.generation.wrapping_add(1)
        );
        assert_eq!(warm.retained.skin.bones, before.skin.bones);
        assert_eq!(warm.retained.skin.model_backpointer, before.model.id);
        assert!(warm.retained.allocations[&before.skin.id].live);
        assert!(!warm.retained.allocations[&before.model.id].live);
        assert_eq!(warm.retained.notification.packed, 0x0eec0003);
        assert!(!warm.retained.allocations[&before.notification.id].live);
        assert_eq!(warm.retained.reset.scale_bits, [0xc479c000; 3]);
        assert_eq!(warm.retained.reset.offset_bits, [0; 3]);
        assert_eq!(warm.retained.reset.dirty, [1; 4]);
        assert_eq!(warm.retained.reset.foreground_stamp, u32::MAX);
        assert_eq!(
            warm.retained.reset.embedded_matrix,
            before.reset.embedded_matrix
        );
        assert_eq!(warm.retained.model.placement, before.model.placement);
        assert!(warm.retained.allocations[&before.animator].live);
        warm.release_animator().unwrap();
        cold_owner.release_animator().unwrap();
        assert_eq!(cold::<Owner>(&cold_owner), warm);
        warm.validate().unwrap();
        assert!(warm.release_animator().is_err());
    }
}

#[test]
fn forged_lifetimes_and_mutated_untouched_fields_fail_validation() {
    let mut x = Owner::from_retained(authored()).unwrap();
    assert!(x.release_animator().is_err());
    x.advance().unwrap();
    for change in 0..4 {
        let mut bad = x.clone();
        match change {
            0 => {
                bad.retained
                    .allocations
                    .get_mut(&bad.retained.skin.id)
                    .unwrap()
                    .live = false
            }
            1 => bad.retained.skin.bones[0].wiggle_records[0][0] ^= 1,
            2 => bad.retained.reset.scale_bits[0] = 0x3f800000,
            _ => bad.events.clear(),
        }
        assert!(bad.validate().is_err());
    }
}

#[test]
fn unsupported_children_and_lost_or_duplicate_owners_fail_before_mutation() {
    for change in 0..7 {
        let mut input = authored();
        match change {
            0 => input.layers[0] = Some(999),
            1 => input.generic[0].binding = Some(998),
            2 => input.notification.children.push(997),
            3 => input.skin.references = 1,
            4 => {
                input.allocations.remove(&input.generic[0].id);
            }
            5 => input.model.name.allocation = input.animator_name.allocation,
            _ => input.reset.invalid_scale_bits = 0x3f800000,
        }
        assert!(Owner::from_retained(input).is_err());
    }
}

#[test]
#[ignore = "private source-derived FIFTH retained owners, no native-runtime equivalence claim"]
fn actual_fifth_source_derived_effects_and_cold_cuts() -> Result<()> {
    use sha2::{Digest, Sha256};
    let path = std::path::PathBuf::from(
        std::env::var_os("SIMS_OBSERVED_GRAPHICS_INPUT")
            .ok_or_else(|| anyhow::anyhow!("input required"))?,
    );
    let bytes = std::fs::read(path)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes))
            == "7c3226d9a2df255e8e9519a16cf0e0e23bea8953440f84d9f4aaa5c260b71969",
        "source-derived retained input changed"
    );
    let input: Vec<Retained> = serde_json::from_slice(&bytes)?;
    ensure!(input.len() == 2, "wrong observed owner count");
    for input in input {
        ensure!(
            input.skin.references == 0
                && input.skin.generation == 1
                && input.reset.invalid_scale_bits == 0xc479c000,
            "wrong observed pre-clear state"
        );
        let mut warm = Owner::from_retained(input)?;
        let mut restored = cold(&warm);
        let mut cuts = 0;
        while !warm.complete() {
            warm.advance()?;
            restored.advance()?;
            restored = cold(&restored);
            restored.validate()?;
            ensure!(warm == restored, "retained graphics cold difference");
            cuts += 1;
            ensure!(cuts <= 32, "teardown did not finish");
        }
        ensure!(
            warm.retained.skin.references == u32::MAX
                && warm.retained.skin.generation == 2
                && warm.retained.reset.scale_bits == [0xc479c000; 3],
            "source-derived effects differ"
        );
        warm.release_animator()?;
        restored.release_animator()?;
        ensure!(warm == cold(&restored), "outer release cold difference");
        println!(
            "{}",
            serde_json::json!({"scope":"Source-derived direct584898 teardown only; intervening resize refresh not replayed", "actor":warm.retained.actor,"cold_service_cuts":cuts,"skin":warm.retained.skin.id,"skin_references":warm.retained.skin.references,"skin_generation":warm.retained.skin.generation,"events":warm.events,"outer_animator_released":warm.outer_released,"strict_parity_ticks_added":0})
        );
    }
    Ok(())
}
