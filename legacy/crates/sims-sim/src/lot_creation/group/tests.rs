use super::*;
use crate::{Snapshot, lot_selection};
use std::collections::BTreeSet;

fn fixture(graphics: bool) -> (Program, Simulation, Definition) {
    let (mut p, mut s) = crate::lot_runtime::tests::fixture();
    let m = p.modules.get_mut("objects").unwrap();
    for (number, offset) in [(100, -1), (10, 0), (11, 256), (12, 1)] {
        let mut w = m.object_definition_words[&1].clone();
        w[10] = 7;
        w[11] = offset;
        w[14] = number as i16;
        w[18] = if number == 100 { 999 } else { 22 };
        w[63] = i16::from(number == 12);
        w[102] = i16::from(graphics);
        m.object_definition_words.insert(number, w);
        m.object_definition_order.push(number);
        let mut functions = m.object_functions[&1].clone();
        functions[0].action = 4100;
        m.object_functions.insert(number, functions);
    }
    let mut init = m.routines[&4096].clone();
    init.body.instructions = vec![sims_formats::behavior::Instruction {
        opcode: 2,
        on_true: 254,
        on_false: 255,
        operands: [41, 0, 0xf7, 0xfc, 0, 5, 3, 7], //Set each part's price to-777.
    }];
    m.routines.insert(4100, init);
    s.architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    s.vm.world.scheduler_storage = Some(Default::default());
    s.vm.world.refresh = Some(Default::default());
    s.vm.world
        .globals
        .extend([(20, 0), (8, 1997), (7, 3), (1, 12), (0, 13), (5, 25)]);
    for id in 1..=4 {
        let mut o = s.vm.world.objects.remove(&id).unwrap();
        o.data.insert(43, 2);
        o.native_flags = Some(0x240);
        s.vm.world.register_scheduler_object(id, &mut o).unwrap();
        s.vm.world.objects.insert(id, o);
        s.vm.world.refresh.as_mut().unwrap().draw.insert(
            id,
            sims_vm::refresh::DrawState::constructed(s.vm.world.positions[&id].level),
        );
        if !s.vm.object_threads.contains_key(&id) {
            s.vm.start_object_lifecycle(&p, id).unwrap();
        }
    }
    (
        p,
        s,
        Definition {
            source: "objects".into(),
            object: 100,
            guid: 100,
            slots: 0,
        },
    )
}
fn cold(s: &Simulation, p: &Program) -> Simulation {
    serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&Snapshot::capture(p, s)).unwrap())
        .unwrap()
        .restore(p)
        .unwrap()
}
#[test]
fn direct_group_factory_retains_each_part_before_graphics_and_cold_resumes_init_and_e2() {
    for graphics in [false, true] {
        let (p, initial, d) = fixture(graphics);
        let mut finals = vec![];
        for resumed in [false, true] {
            let mut s = initial.clone();
            let mut ordinary = BTreeSet::new();
            s.begin_native_object_factory(&p, d.clone()).unwrap();
            let mut notified = vec![];
            let mut graphical = vec![];
            let mut constructed_prefixes = BTreeSet::new();
            let result = loop {
                let (_, result) = s
                    .advance_native_object_factory(&p, &ordinary, if resumed { 1 } else { 1000 })
                    .unwrap();
                if let Some(id) = result {
                    break id.unwrap();
                }
                if resumed {
                    let ids = s.native_selection.as_ref().unwrap().constructing_objects();
                    if !ids.is_empty() {
                        constructed_prefixes.insert(ids.len());
                        assert!(ids.iter().all(|id| !s.vm.object_threads.contains_key(id)));
                        let mut orphan = Snapshot::capture(&p, &s);
                        orphan.state.native_selection = None;
                        assert!(orphan.restore(&p).is_err());
                    }
                    s = cold(&s, &p);
                }
                match s.pending_native_selection() {
                    Some((r @ lot_selection::Request::FactoryGraphic(id), false)) => {
                        //431aaf has published its bounds before the graphic
                        //service and before date/Init. Keep them through cold restore.
                        assert_eq!(
                            s.native_lot.as_ref().unwrap().placement.bounds[&id],
                            [-12, -12, -20, -20]
                        );
                        assert_eq!(s.vm.world.objects[&id].data[&45], 0);
                        assert_eq!(
                            s.vm.world.objects[&id].multitile.as_ref().unwrap().origin,
                            [0; 3]
                        );
                        assert!(s.complete_selection_factory_graphic(&p, id).is_err());
                        s.start_selection_graphic(r).unwrap();
                        assert!(s.complete_selection_factory_graphic(&p, id + 100).is_err());
                        if resumed {
                            s = cold(&s, &p);
                        }
                        s.complete_selection_factory_graphic(&p, id).unwrap();
                        graphical.push(id);
                        ordinary.insert(id);
                        assert_eq!(s.vm.world.objects[&id].data[&45], 1997);
                        assert!(s.complete_selection_factory_graphic(&p, id).is_err());
                    }
                    Some((lot_selection::Request::CreationNotification(id), false)) => {
                        let parts = sims_vm::multitile::chain(&s.vm.world, id).unwrap();
                        assert_eq!(parts, [5, 6, 7]);
                        for id in parts {
                            assert_eq!(s.vm.world.objects[&id].data[&41], -777);
                            assert_ne!(s.vm.world.objects[&id].idle.as_ref().unwrap().flags & 1, 0);
                        }
                        s.start_indicator_created(id).unwrap();
                        if resumed {
                            s = cold(&s, &p);
                        }
                        assert!(s.complete_indicator_created(id + 1).is_err());
                        s.complete_indicator_created(id).unwrap();
                        notified.push(id);
                    }
                    None => (),
                    other => panic!("unexpected group factory request {other:?}"),
                }
            };
            assert_eq!(result, 5);
            assert_eq!(notified, [5]);
            assert_eq!(graphical, if graphics { vec![5, 6, 7] } else { vec![] });
            assert_eq!(
                s.vm.world.objects[&5].definition.as_ref().unwrap().object,
                12
            );
            for id in [5, 6, 7] {
                assert!(s.unplaced.contains(&id));
                assert_eq!(s.native_lot.as_ref().unwrap().placement.links.next[&id], 0);
                assert_eq!(
                    s.native_lot.as_ref().unwrap().placement.bounds[&id],
                    [-12, -12, -20, -20]
                );
            }
            if resumed {
                assert_eq!(constructed_prefixes, BTreeSet::from([1, 2, 3]));
            }
            assert_eq!(s.vm.tick, initial.vm.tick);
            assert_eq!(s.vm.rng_state, initial.vm.rng_state);
            assert_eq!(cold(&s, &p), s);
            finals.push(s);
        }
        assert_eq!(finals[0], finals[1]);
    }
}
