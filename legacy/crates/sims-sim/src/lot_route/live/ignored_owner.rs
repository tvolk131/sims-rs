//! Independent pointer/footprint-word ownership for potential clearance.
use super::*;

#[test]
fn potential_clearance_ignored_pointer_is_independent_of_footprint_word() {
    //Authored forwarding/restore-validation case, not a native World import.
    for pointer in [None, Some(3)] {
        for word in [0, 0x1234] {
            let (p, mut s) = crate::lot_runtime::tests::fixture();
            s.vm.world.objects.get_mut(&2).unwrap().person =
                Some((0..100).map(|i| (i, 0)).collect());
            s.visit = Some((0, None));
            let descriptor = sims_vm::routing::Descriptor([
                0,
                0,
                0,
                1,
                1,
                0,
                0,
                0x801,
                u32::MAX,
                16,
                16,
                16,
                0.1875_f32.to_bits(),
                (-3_i32) as u32,
                16,
            ]);
            let route = RouteRequest {
                id: 1,
                origin: sims_vm::Location {
                    source: "authored-component".into(),
                    bhav: 0,
                    instruction: 0,
                },
                caller: 2,
                target: 3,
                destination: sims_vm::routing::Destination::Slot {
                    source: "authored-component".into(),
                    chunk: 0,
                    index: 0,
                    version: 10,
                    slot: descriptor.slot(),
                },
                no_failure_trees: false,
            };
            let q = Execution::begin_potential(
                &mut s,
                0,
                &route,
                Potential {
                    ignored_person: pointer,
                    ignore_people: false,
                    ignored_word: word,
                    blocked_object: 0,
                    retained_result: 0,
                },
            )
            .unwrap();
            assert_eq!(q.input.ignored, pointer);
            assert_eq!(s.vm.world.objects[&2].person.as_ref().unwrap()[&73], word);
            q.validate_request(&s, &p, &route).unwrap();
            let restored: Execution =
                serde_json::from_slice(&serde_json::to_vec(&q).unwrap()).unwrap();
            assert_eq!(q, restored);
            restored.validate_request(&s, &p, &route).unwrap();
            let mut forged = restored;
            forged.input.ignored = if pointer.is_some() { None } else { Some(3) };
            assert_eq!(
                forged.validate_request(&s, &p, &route).unwrap_err(),
                "invalid native route prefix binding"
            );
        }
    }
}
