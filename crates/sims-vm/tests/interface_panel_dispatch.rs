use sims_vm::interface::panel_dispatch::{Inputs, Operation as O, Request, Update};
fn input() -> Inputs {
    Inputs {
        mode: 2,
        dirty: 2,
        controls: Some(10),
        build: Some(20),
        buy: Some(30),
        people: Some(40),
        options: Some(50),
        camera: Some(60),
    }
}

#[test]
fn panel_dispatch_restores_every_child_and_rereads_mode_between_children() {
    let mut i = input();
    let mut f = Update::new(99);
    let mut got = vec![];
    for _ in 0..10 {
        let Some(r) = f.advance(i).unwrap() else {
            break;
        };
        got.push(r);
        let save = serde_json::to_vec(&f).unwrap();
        f = serde_json::from_slice(&save).unwrap();
        assert_eq!(f.advance(i).unwrap(), Some(r));
        let mut wrong = r;
        wrong.child = Some(777);
        assert!(f.acknowledge(wrong).is_err());
        match r.operation {
            O::Controls => i.mode = 0,
            O::Build => i.mode = 1,
            O::Buy => i.mode = 2,
            O::People => {
                i.dirty = 2;
                i.people = Some(41);
                i.mode = 3;
            }
            O::InvalidatePeople => i.mode = 3,
            O::Options => i.mode = 4,
            _ => (),
        }
        f.acknowledge(r).unwrap();
    }
    assert!(f.complete());
    assert_eq!(
        got,
        vec![
            Request {
                operation: O::Controls,
                child: Some(10),
                state: Some(99)
            },
            Request {
                operation: O::Build,
                child: Some(20),
                state: Some(99)
            },
            Request {
                operation: O::Buy,
                child: Some(30),
                state: Some(99)
            },
            Request {
                operation: O::People,
                child: Some(40),
                state: Some(99)
            },
            Request {
                operation: O::InvalidatePeople,
                child: Some(41),
                state: None
            },
            Request {
                operation: O::Options,
                child: Some(50),
                state: Some(99)
            },
            Request {
                operation: O::Camera,
                child: Some(60),
                state: Some(99)
            }
        ]
    );
}

#[test]
fn absent_panels_and_post_people_invalidation_have_distinct_gates() {
    for mode in [-1, 0, 1, 2, 3, 4, 5] {
        let mut i = input();
        i.mode = mode;
        let mut f = Update::new(99);
        let mut got = vec![];
        while let Some(r) = f.advance(i).unwrap() {
            got.push(r.operation);
            f.acknowledge(r).unwrap();
        }
        let mut expected = vec![
            O::Controls,
            if mode == 0 { O::Build } else { O::HideBuild },
            if mode == 1 { O::Buy } else { O::HideBuy },
        ];
        match mode {
            2 => expected.extend([O::People, O::InvalidatePeople]),
            3 => expected.push(O::Options),
            4 => expected.push(O::Camera),
            _ => (),
        }
        assert_eq!(got, expected);
    }
    let mut i = input();
    i.controls = None;
    i.build = None;
    i.buy = None;
    i.options = None;
    i.camera = None;
    let mut f = Update::new(99);
    let r = f.advance(i).unwrap().unwrap();
    assert_eq!(r.child, None);
    assert_eq!(r.operation, O::Controls);
    f.acknowledge(r).unwrap();
    let r = f.advance(i).unwrap().unwrap();
    assert_eq!(r.operation, O::People);
    f.acknowledge(r).unwrap();
    i.people = None;
    i.mode = 1; // Do not repeat the earlier presence/mode gate.
    let r = f.advance(i).unwrap().unwrap();
    assert_eq!(r.operation, O::InvalidatePeople);
    assert_eq!(r.child, None);
    f.acknowledge(r).unwrap();
    assert_eq!(f.advance(i).unwrap(), None);
}

#[test]
fn people_callback_can_clear_dirty_without_dispatching_invalidation() {
    let mut i = input();
    let mut f = Update::new(99);
    let mut got = vec![];
    while let Some(r) = f.advance(i).unwrap() {
        got.push(r.operation);
        if r.operation == O::People {
            i.dirty = 0;
        }
        f.acknowledge(r).unwrap();
    }
    assert_eq!(got, vec![O::Controls, O::HideBuild, O::HideBuy, O::People]);
}
