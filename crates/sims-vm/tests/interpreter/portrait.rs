//! A dialog's portrait (0x43601c, 0x436279, 0x4e92a8, 0x4e8c5d,
//! 0x4e9652): the Dialog primitive (opcode 36) has the interface build
//! the portrait of the person type its icon mode names, which makes a
//! person of that type and removes it at once when nobody of it is on the
//! lot (`Machine::dialog_portrait`). Synthetic; no game data.

use crate::create_person::visitors;
use crate::factory::WIDGET;
use crate::support::*;

/// Dialog (opcode 36).
const DIALOG: u16 = 36;

/// Runs, as the lamp's tree 4096 with stack object `stack_object`, a
/// one-button dialog that only informs (operand 7 bit 0) with icon mode
/// `mode` (operand 7 bits 1 to 3).
fn inform(fx: &mut Fixture, mode: u8, stack_object: i16) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 1 | mode << 1],
        )]),
    );
    fx.run_with(LAMP, 4096, stack_object, &[], 1000)
}

/// The lot without its dialog records, portraits and frame storage: what
/// a dialog that builds no portrait leaves as it was.
fn rest(fx: &Fixture) -> [u8; 32] {
    let mut world = fx.world.clone();
    world.dialogs.records.clear();
    world.portraits.clear();
    hash_without_frames(&world)
}

/// [`rest`] after the same dialog run with icon mode 1, which shows no
/// portrait, on a copy of the lot.
fn as_with_no_portrait(fx: &Fixture, stack_object: i16) -> [u8; 32] {
    let mut copy = Fixture {
        program: fx.program.clone(),
        catalog: fx.catalog.clone(),
        world: fx.world.clone(),
        stream: fx.stream,
    };
    assert_eq!(inform(&mut copy, 1, stack_object), Ok(true));
    rest(&copy)
}

#[test]
fn a_neighbors_portrait_makes_a_person_of_its_type_and_removes_it_once() {
    // Mode 2: the type of the neighbor record the stack object names
    // (0x425afe, record +8). Nobody of it is on the lot, so a person is
    // made and removed at once (0x4e9652, 0x4e8fdb).
    let (mut fx, _, record) = visitors();
    let objects = fx.world.objects.clone();
    let before = as_with_no_portrait(&fx, record);
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert_eq!(
        fx.world.objects.keys().collect::<Vec<_>>(),
        objects.keys().collect::<Vec<_>>()
    );
    assert!(
        fx.world.skeleton_files.contains("widget.bcf"),
        "the person made read its skeleton"
    );
    assert_ne!(rest(&fx), before);
    // The type has its image now: a second dialog builds nothing.
    fx.world.dialogs.records.clear();
    let after = rest(&fx);
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert_eq!(rest(&fx), after);
}

#[test]
fn a_portrait_of_a_type_someone_on_the_lot_has_makes_nobody() {
    // 0x4e8c5d draws the first person of the type on the lot (0x4e9608).
    // Mode 0: the stack object's own type.
    let mut fx = Fixture::new();
    fx.world.people = vec![SIM];
    let before = as_with_no_portrait(&fx, SIM as i16);
    assert_eq!(inform(&mut fx, 0, SIM as i16), Ok(true));
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    assert_eq!(rest(&fx), before);
}

#[test]
fn modes_with_no_person_build_no_portrait() {
    // Mode 0 on an object that is not a person, and a stack object that
    // names nothing; mode 2 with no record.
    let (fx, _, record) = visitors();
    for (mode, stack_object) in [(0, LAMP as i16), (0, 999), (2, record + 1)] {
        let mut fx = Fixture {
            program: fx.program.clone(),
            catalog: fx.catalog.clone(),
            world: fx.world.clone(),
            stream: fx.stream,
        };
        let before = as_with_no_portrait(&fx, stack_object);
        assert_eq!(inform(&mut fx, mode, stack_object), Ok(true), "{mode}");
        assert!(fx.world.portraits.is_empty());
        assert_eq!(rest(&fx), before, "{mode} {stack_object}");
    }
}

#[test]
fn a_portrait_the_port_cannot_tell_was_built_fails_loudly() {
    // A type of the household with nobody of it on the lot: the household
    // panel may have built it (not kept).
    let (mut fx, _, record) = visitors();
    fx.world.globals[9] = 7;
    fx.world.families.push(sims_sim::neighbors::Family {
        id: 7,
        house: 0,
        members: vec![WIDGET],
        friends: 0,
        stale: false,
    });
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(
        fault.contains("0x4e9652") && fault.contains("household"),
        "{fault}"
    );
    assert!(fx.world.portraits.is_empty());
    // A type made while the game ran may not be in the interface's table.
    let (mut fx, _, record) = visitors();
    fx.world.characters.made.insert(
        WIDGET,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 0,
            strings: Default::default(),
        },
    );
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("0x6f2698"), "{fault}");
}

#[test]
fn a_portrait_whose_person_runs_code_when_put_down_fails_loudly() {
    // The person made is put on a tile (0x45e4e9, 0x433340), which asks
    // it its function 9 (0x433580), and its function 6 when the walls
    // beside the tile differ from its own (0x434fac, 0x435110). The tile
    // is not modeled.
    for (function, at) in [
        (9, "0x433580"),
        (6, "0x435110"),
        (26, "0x433612"),
        (25, "0x433612"),
        (10, "0x433148"),
    ] {
        let (mut fx, file, record) = visitors();
        let objects: Vec<u16> = fx.world.objects.keys().copied().collect();
        let library = fx.program.libraries.get_mut(&file).unwrap();
        for table in library.callbacks.values_mut() {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
            table[function].action = 4200;
        }
        let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
        assert!(fault.contains(at) && fault.contains("tree 4200"), "{fault}");
        assert!(fx.world.portraits.is_empty());
        // The build ran on a copy of the lot: the person made is not left
        // behind.
        assert_eq!(
            fx.world.objects.keys().copied().collect::<Vec<_>>(),
            objects
        );
    }
}

#[test]
fn a_portrait_whose_put_depends_on_its_tile_fails_loudly() {
    // Data word 40 bit 0x400 marks the room the person lands in as not
    // counted (0x433340).
    let (mut fx, file, record) = visitors();
    // The widget's Init (tree 4096, `crate::find_location::placeable`),
    // with the bit set.
    fx.add(
        file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
            assign(scope::MY_DATA, 40, scope::LITERAL, 0x400, RETURN_TRUE),
        ]),
    );
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("bit 0x400"), "{fault}");
    assert!(fx.world.portraits.is_empty());
    // Data word 7 marks the room it is put in and taken from as not
    // counted for its light (0x433ed8, 0x4412c6).
    let (mut fx, file, record) = visitors();
    fx.add(
        file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
            assign(scope::MY_DATA, 7, scope::LITERAL, 1, RETURN_TRUE),
        ]),
    );
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("data word 7"), "{fault}");
    assert!(fx.world.portraits.is_empty());
    // A person of kind 0 or 1 marks the indoor room it lands in as
    // occupied (0x45e43d), which its removal leaves: it fails while an
    // indoor room that is not a pool is not marked. One already marked,
    // one outdoors and a pool are left as they are, and so is any room by
    // a pet (kind above 1). The person's words come from its neighbor
    // record (`crate::person_factory::equip`).
    for (kind, outdoors, pool, occupied, fails) in [
        (0, false, false, false, true),
        (1, false, false, false, true),
        (2, false, false, false, false),
        (0, false, false, true, false),
        (0, true, false, false, false),
        (0, false, true, false, false),
    ] {
        let (mut fx, _, record) = visitors();
        fx.world.rooms.rooms.clear();
        fx.world.neighbors.slots[record as usize - 1]
            .as_mut()
            .unwrap()
            .data[65] = kind;
        fx.world.rooms.rooms.insert(
            1,
            sims_sim::rooms::Room {
                outdoors,
                pool,
                occupied,
                ..Default::default()
            },
        );
        let ran = inform(&mut fx, 2, record);
        if fails {
            let fault = ran.unwrap_err().to_string();
            assert!(fault.contains("0x468802"), "{kind}: {fault}");
        } else {
            assert_eq!(ran, Ok(true), "{kind} {outdoors} {pool} {occupied}");
        }
    }
    // The rooms upstairs are not the search's: it is on the ground level.
    let (mut fx, _, record) = visitors();
    fx.world
        .rooms
        .rooms
        .insert(9, sims_sim::rooms::Room::default());
    fx.world.rooms.map[1].fill(9);
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    // No tile takes it: the original removes it unput (0x4e9706), and
    // builds again (0x4e938c) unless the type has image 2002. On a lot
    // with no inside the start is 1,1.
    let (mut fx, _, record) = visitors();
    fx.world.size = 2;
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(
        fault.contains("0x4e9706") && fault.contains("from 1 of the starts (1,1)"),
        "{fault}"
    );
}

/// [`inform`] with the house view's centre at `centre`.
fn inform_from(
    fx: &mut Fixture,
    mode: u8,
    stack_object: i16,
    centre: Option<[i8; 2]>,
) -> Result<bool, sims_vm::Fault> {
    inform_hearing(fx, mode, stack_object, centre, &Default::default())
}

/// [`inform_from`], with the sounds played going to `sounds`.
fn inform_hearing(
    fx: &mut Fixture,
    mode: u8,
    stack_object: i16,
    centre: Option<[i8; 2]>,
    sounds: &std::cell::RefCell<Vec<sims_vm::SoundEvent>>,
) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 1 | mode << 1],
        )]),
    );
    let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.view_centre = centre;
    interpreter.sounds = Some(sounds);
    interpreter.run_tree(&mut fx.world, LAMP, 4096, stack_object, &[], 1000)
}

/// The fixture's lot, 6 tiles wide, with the tiles whose first
/// coordinate is below 3 outdoors (room 2) and the rest an indoor room
/// nobody is in (1); the last neighbor record's person is of kind 0
/// (word 65).
fn indoors_and_out(fx: &mut Fixture) {
    let record = fx.world.neighbors.slots.len();
    fx.world.neighbors.slots[record - 1].as_mut().unwrap().data[65] = 0;
    fx.world.rooms.rooms.clear();
    fx.world
        .rooms
        .rooms
        .insert(1, sims_sim::rooms::Room::default());
    fx.world.rooms.rooms.insert(
        2,
        sims_sim::rooms::Room {
            outdoors: true,
            ..Default::default()
        },
    );
    for a in 0..64 {
        for b in 0..64 {
            fx.world.rooms.map[0][a * 64 + b] = if a < 3 { 2 } else { 1 };
        }
    }
}

#[test]
fn a_portrait_searches_from_the_view_centre_when_it_is_given() {
    // On the fixture's lot, 6 tiles wide, tiles with a first coordinate
    // below 3 are outdoors (room 2), the rest an indoor room nobody is in
    // (1). A person of kind 0 put indoors
    // marks the room as occupied (0x45e43d), which its removal leaves. The
    // person's kind comes from its neighbor record (word 65).
    let rooms = indoors_and_out;
    // With no centre, some start ends indoors.
    let (mut fx, _, record) = visitors();
    rooms(&mut fx);
    let fault = inform_from(&mut fx, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x468802"), "{fault}");
    // From a centre outdoors the search ends outdoors, in the start's
    // room, wherever outdoors it starts (a start outside the border is
    // taken inside it), and the lot ends the same.
    let mut ends = Vec::new();
    for centre in [[1, 1], [2, 4], [0, 0], [2, 60]] {
        let (mut fx, _, record) = visitors();
        rooms(&mut fx);
        assert_eq!(inform_from(&mut fx, 2, record, Some(centre)), Ok(true));
        assert_eq!(fx.world.portraits, [WIDGET].into());
        ends.push(hash_without_frames(&fx.world));
    }
    assert!(ends.windows(2).all(|w| w[0] == w[1]));
    // From a centre indoors it ends indoors.
    let (mut fx, _, record) = visitors();
    rooms(&mut fx);
    let fault = inform_from(&mut fx, 2, record, Some([4, 2]))
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x468802"), "{fault}");
    // From a centre no tile is found from, it fails.
    let (mut fx, _, record) = visitors();
    fx.world.size = 2;
    let fault = inform_from(&mut fx, 2, record, Some([7, 7]))
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains("from the view centre 7,7 no tile is found") && fault.contains("0x4e9706"),
        "{fault}"
    );
}

/// An object type with a function 5 (may overlap, 0x4337f6), tree 4300.
const OBSTACLE: u32 = 0x7777;

/// What the obstacle's function 5 does.
#[derive(Clone, Copy)]
enum Callback {
    Writes,
    Plays,
    Silent,
}

/// The visitors' lot ([`visitors`]) with an [`OBSTACLE`] at 3,3 whose
/// function 5 does `callback`: the lot, the widget's file, the neighbor
/// record and the obstacle.
fn obstacle(callback: Callback) -> (Fixture, usize, i16, u16) {
    let (mut fx, widget, record) = visitors();
    let file = fx.add_type(OBSTACLE, 4, 4096, 4097);
    fx.add(
        file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
        ]),
    );
    fx.add(file, 4097, sleeper());
    crate::visits::function(&mut fx, file, 5, 4300);
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .sounds
        .insert(5, "bump".into());
    fx.add(
        file,
        4300,
        tree(vec![match callback {
            Callback::Writes => assign(scope::MY_DATA, 50, scope::LITERAL, 1, RETURN_FALSE),
            Callback::Plays => ins(23, RETURN_TRUE, ERROR, [5, 0, 0, 0, 0, 0, 0, 0]),
            Callback::Silent => ins(23, RETURN_TRUE, ERROR, [6, 0, 0, 0, 0, 0, 0, 0]),
        }]),
    );
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let id = interpreter.create(&mut fx.world, OBSTACLE).unwrap();
    let cell = (3 * 64 + 3) * 2;
    let grid = &mut fx.world.arrays.get_mut(&3).unwrap().data;
    grid[cell..cell + 2].copy_from_slice(&id.to_le_bytes());
    let position = [3 * 16 + 8, 3 * 16 + 8, 1];
    let room = fx.world.room_at(position).unwrap();
    let obstacle = fx.world.objects.get_mut(&id).unwrap();
    obstacle.position = position;
    obstacle.data[29] = room;
    (fx, widget, record, id)
}

#[test]
fn a_callback_the_search_runs_must_change_nothing_when_no_centre_is_given() {
    // The obstacle stands at 3,3; the search's place test beside it asks
    // its function 5 (0x433a1a), which changes the lot. Which tiles the
    // original tries depends on the start, so with no view centre the
    // portrait fails, and leaves the lot as it was.
    // Its function 5 sets its data word 50 (and returns false, which the
    // object keeps as its last result), or plays a sound and returns true
    // as its last result was, or plays one its library does not have and
    // returns true. Even that last one changes the lot: a run leaves its
    // frame's words in the object's stack storage (`FrameStorage`), so
    // with no view centre any callback the search runs stops it, unless
    // the storage held the same words already (when only the cells, such
    // as a sound, can show it).
    let setup = |callback: Callback| {
        let (fx, _, record, id) = obstacle(callback);
        (fx, record, id)
    };
    let (mut fx, record, id) = setup(Callback::Writes);
    let objects: Vec<u16> = fx.world.objects.keys().copied().collect();
    let fault = inform_from(&mut fx, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains(&format!("object {id}'s function 5 (tree 4300)")),
        "{fault}"
    );
    assert_eq!(
        fx.world.objects.keys().copied().collect::<Vec<_>>(),
        objects
    );
    assert_eq!(fx.world.objects[&id].data[50], 0);
    // From a centre beside it, the search asks it as the original does,
    // and what it does stays.
    let (mut fx, record, id) = setup(Callback::Writes);
    assert_eq!(inform_from(&mut fx, 2, record, Some([3, 4])), Ok(true));
    assert_eq!(fx.world.objects[&id].data[50], 1);
    // A sound is a change to the machine's cells: the same.
    let (mut fx, record, id) = setup(Callback::Plays);
    let heard = Default::default();
    let fault = inform_hearing(&mut fx, 2, record, None, &heard)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains(&format!("object {id}'s function 5 (tree 4300)")),
        "{fault}"
    );
    assert!(heard.borrow().is_empty());
    let (mut fx, record, id) = setup(Callback::Plays);
    let heard = Default::default();
    assert_eq!(
        inform_hearing(&mut fx, 2, record, Some([3, 4]), &heard),
        Ok(true)
    );
    // (Other sounds the inform makes, such as a stop, come after it.)
    assert_eq!(
        heard.into_inner().first(),
        Some(&sims_vm::SoundEvent::Play {
            object: id as i16,
            name: "bump".into()
        })
    );
    // A callback that plays nothing and returns its last result still
    // leaves its frame in the obstacle's stack storage.
    let (mut fx, record, id) = setup(Callback::Silent);
    assert!(fx.world.objects[&id].last_result);
    let storage = fx.world.frame_storage[&id].clone();
    let fault = inform_from(&mut fx, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains(&format!("object {id}'s function 5 (tree 4300)")),
        "{fault}"
    );
    assert_eq!(
        format!("{:?}", fx.world.frame_storage[&id]),
        format!("{storage:?}")
    );
    // With the storage holding the words its run leaves already, only the
    // sound it plays shows the change.
    let (mut fx, record, id) = setup(Callback::Plays);
    let heard = Default::default();
    assert_eq!(
        inform_hearing(&mut fx, 2, record, Some([3, 4]), &heard),
        Ok(true)
    );
    let used = fx.world.frame_storage[&id].clone();
    let (mut fx, record, id) = setup(Callback::Plays);
    fx.world.frame_storage.insert(id, used);
    let heard = Default::default();
    let fault = inform_hearing(&mut fx, 2, record, None, &heard)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains(&format!("object {id}'s function 5 (tree 4300)")),
        "{fault}"
    );
    assert!(heard.borrow().is_empty());
    let (mut fx, record, id) = setup(Callback::Silent);
    assert_eq!(inform_from(&mut fx, 2, record, Some([3, 4])), Ok(true));
    assert_ne!(
        format!("{:?}", fx.world.frame_storage[&id]),
        format!("{storage:?}")
    );
}

/// Gives the person's function `function` tree `tree`, in every callback
/// table of the widget's file.
fn widget_function(fx: &mut Fixture, file: usize, function: usize, tree: u16) {
    let library = fx.program.libraries.get_mut(&file).unwrap();
    for table in library.callbacks.values_mut() {
        if table.len() <= function {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
        }
        table[function].action = tree;
    }
}

#[test]
fn with_a_view_centre_the_persons_own_function_5_runs_and_its_words_are_read_after() {
    // The person's function 5 is the search's to ask, with the object in
    // its way as the stack object (0x433a1a): from a centre beside the
    // obstacle it runs as in the original, then the obstacle's.
    let (mut fx, widget, record, id) = obstacle(Callback::Writes);
    fx.add(
        widget,
        4200,
        tree(vec![assign(
            scope::MY_DATA,
            50,
            scope::LITERAL,
            1,
            RETURN_FALSE,
        )]),
    );
    widget_function(&mut fx, widget, 5, 4200);
    assert_eq!(inform_from(&mut fx, 2, record, Some([3, 4])), Ok(true));
    assert_eq!(fx.world.objects[&id].data[50], 1);
    assert_eq!(fx.world.portraits, [WIDGET].into());
    // What it does is read after the search: data word 7 (0x433ed8) and
    // data word 40 bit 0x400 (0x433340), set by it, fail the put.
    for (word, value, said) in [(7, 1, "data word 7"), (40, 0x400, "bit 0x400")] {
        let (mut fx, widget, record, _) = obstacle(Callback::Writes);
        fx.add(
            widget,
            4200,
            tree(vec![assign(
                scope::MY_DATA,
                word,
                scope::LITERAL,
                value,
                RETURN_FALSE,
            )]),
        );
        widget_function(&mut fx, widget, 5, 4200);
        let fault = inform_from(&mut fx, 2, record, Some([3, 4]))
            .unwrap_err()
            .to_string();
        assert!(fault.contains(said), "{word}: {fault}");
        assert!(fx.world.portraits.is_empty());
    }
}

#[test]
fn a_portrait_put_that_takes_the_mark_off_the_room_it_was_last_seen_in_fails_loudly() {
    // The person's Init names room 1 in its data word 29, and its first
    // tick, which the factory runs, notes that as the room it was last
    // seen in (0x45e43d). Put outdoors, it then takes room 1's mark off
    // when nobody else was last seen there (0x46886c, 0x468802), and the
    // removal leaves it so.
    let setup = |others: bool| {
        let (mut fx, file, record) = visitors();
        indoors_and_out(&mut fx);
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.add(
            file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
                assign(scope::MY_DATA, 29, scope::LITERAL, 1, RETURN_TRUE),
            ]),
        );
        if others {
            fx.world.person_ticks.insert(
                SIM,
                sims_sim::PersonTick {
                    room: 1,
                    ..sims_sim::PersonTick::CONSTRUCTED
                },
            );
        }
        (fx, record)
    };
    let (mut fx, record) = setup(false);
    let rooms = fx.world.rooms.clone();
    let fault = inform_from(&mut fx, 2, record, Some([1, 1]))
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains("room 2") && fault.contains("0x468802"),
        "{fault}"
    );
    assert_eq!(fx.world.rooms, rooms);
    // With no centre, the same from every start outdoors.
    let (mut fx, record) = setup(false);
    let fault = inform_from(&mut fx, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x468802"), "{fault}");
    // Someone else last seen in room 1 keeps its mark.
    let (mut fx, record) = setup(true);
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert!(fx.world.rooms.rooms[&1].occupied);
}

#[test]
fn a_portrait_put_on_a_lot_with_no_indicators_fails_loudly() {
    // The interface's work for an object come from outside (0x44752e)
    // makes the lot's indicators when it has none (0x445e4a). Their Init
    // runs while the original's person stands on its tile, in its room;
    // here it does not, so the put fails.
    let (mut fx, _, record) = visitors();
    assert_ne!(fx.world.indicators[0], 0);
    fx.world.indicators = [0, 0];
    let objects: Vec<u16> = fx.world.objects.keys().copied().collect();
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("0x445e4a"), "{fault}");
    assert!(fx.world.portraits.is_empty());
    assert!(fx.world.objects.keys().copied().eq(objects));
    assert_eq!(fx.world.indicators, [0, 0]);
}

#[test]
fn a_portrait_build_that_fails_leaves_the_machines_cells_as_they_were() {
    // The person's Init plays a sound, and the put then fails (data word
    // 40 bit 0x400, 0x433340): the sound is not kept, as the person is
    // not. Without the bit the build is kept, the sound with it.
    for bit in [0x400, 0] {
        let (mut fx, file, record) = visitors();
        fx.program
            .libraries
            .get_mut(&file)
            .unwrap()
            .sounds
            .insert(5, "hello".into());
        fx.add(
            file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
                ins(23, 3, ERROR, [5, 0, 0, 0, 0, 0, 0, 0]),
                assign(scope::MY_DATA, 40, scope::LITERAL, bit, RETURN_TRUE),
            ]),
        );
        let heard = Default::default();
        let ran = inform_hearing(&mut fx, 2, record, Some([1, 1]), &heard);
        let heard = heard.into_inner();
        if bit != 0 {
            let fault = ran.unwrap_err().to_string();
            assert!(fault.contains("bit 0x400"), "{fault}");
            assert!(heard.is_empty(), "{heard:?}");
        } else {
            assert_eq!(ran, Ok(true));
            assert!(
                heard.iter().any(|event| matches!(
                    event,
                    sims_vm::SoundEvent::Play { name, .. } if name == "hello"
                )),
                "{heard:?}"
            );
        }
    }
}

/// The strings of the dialogs in [`text_dialog`]: STR# 301, from 1.
const TEXT_STRINGS: [&str; 5] = [
    "Hello",
    "$Neighbor",
    "$Object",
    "$Object then $Neighbor",
    "$$Object",
];

/// The catalog string table the person's definition names (word 41).
const SIM_NAMES: u16 = 2000;

/// The catalog string table the widget's definition names (word 41).
const WIDGET_NAMES: u16 = 2002;

/// The person (2) as the stack object of a dialog whose message, label
/// and title are entries of [`TEXT_STRINGS`] (0 for none). Neighbor record
/// 2 is of the widget's type, with nobody of it on the lot; the widget's
/// catalog name, which `$Neighbor` is replaced by, is `Widget`. The
/// person's catalog name, which `$Object` is replaced by, is `names`
/// (CTSS entries with their language), or no table at all.
fn text_dialog((label, title, message): (u8, u8, u8), names: Option<Vec<(u8, &str)>>) -> Fixture {
    let (mut fx, widget, record) = visitors();
    let neighbor = fx.world.neighbors.slots[record as usize - 1].clone();
    let slots = &mut fx.world.neighbors.slots;
    slots.resize(slots.len().max(2), None);
    slots[usize::from(SIM) - 1] = neighbor;
    fx.world.people = vec![SIM];
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .strings
        .insert(301, TEXT_STRINGS.map(str::to_owned).to_vec());
    fx.words(SIM_FILE)[41] = SIM_NAMES as i16;
    if let Some(names) = names {
        name(&mut fx, SIM_FILE, SIM_NAMES, names);
    }
    fx.words(widget)[41] = WIDGET_NAMES as i16;
    name(&mut fx, widget, WIDGET_NAMES, vec![(1, "Widget")]);
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, message, label, 0, 0, title, 1],
        )]),
    );
    fx
}

/// Puts catalog string table `table` in file `file`, with `names` (CTSS
/// entries with their language).
fn name(fx: &mut Fixture, file: usize, table: u16, names: Vec<(u8, &str)>) {
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .catalog_strings
        .insert(
            table,
            names.into_iter().map(|(l, t)| (l, t.to_owned())).collect(),
        );
}

/// Gives the widget of a [`text_dialog`] the catalog names `$Neighbor` is
/// replaced by, or none.
fn name_widget(fx: &mut Fixture, names: Option<Vec<(u8, &str)>>) {
    // The widget's file is the one with the "init visitor" tree.
    let widget = *fx
        .program
        .libraries
        .iter()
        .find(|(_, library)| library.names.values().any(|n| n == "init visitor"))
        .unwrap()
        .0;
    fx.program
        .libraries
        .get_mut(&widget)
        .unwrap()
        .catalog_strings
        .remove(&WIDGET_NAMES);
    if let Some(names) = names {
        name(fx, widget, WIDGET_NAMES, names);
    }
}

/// Sets string 1 of the lamp's STR# 301, the message in most dialogs here.
fn set_message(fx: &mut Fixture, text: &str) {
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .strings
        .get_mut(&301)
        .unwrap()[0] = text.to_owned();
}

/// The catalog string table the lamp's definition names (word 41).
const LAMP_NAMES: u16 = 2001;

/// Gives the lamp, which runs the dialogs here, the catalog names `$Me`
/// is replaced by.
fn name_lamp(fx: &mut Fixture, names: Vec<(u8, &str)>) {
    fx.words(LAMP_FILE)[41] = LAMP_NAMES as i16;
    name(fx, LAMP_FILE, LAMP_NAMES, names);
}

/// Makes the lamp's tree 4096 the dialog of a [`text_dialog`] with icon
/// mode `mode` (operand 7 bits 1 to 3), message 1 and operand 1 `extra`.
fn in_mode(fx: &mut Fixture, mode: u8, extra: u8) {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, extra, 1, 0, 0, 0, 0, 1 | mode << 1],
        )]),
    );
}

#[test]
fn a_dialogs_text_can_show_the_stack_objects_neighbor_instead() {
    // Mode 0 shows first the type the formatter (0x44bf1a) found in the
    // record's strings, formatted in the order button (operand 3), title
    // (operand 6), message (operand 2): `$Neighbor` gives the stack
    // object's neighbor record's type, `$Object` the stack object's, and
    // the last found is kept. With neither, the stack object's (0x436308).
    // The names are plain.
    for (strings, shown) in [
        ((0, 0, 1), SIM_GUID),
        ((0, 0, 2), WIDGET),
        ((2, 3, 0), SIM_GUID),
        ((3, 0, 2), WIDGET),
        ((0, 0, 4), WIDGET),
    ] {
        let mut fx = text_dialog(strings, Some(vec![(1, "Grim")]));
        let ran = fx.run_with(LAMP, 4096, SIM as i16, &[], 1000);
        assert_eq!(ran, Ok(true), "{strings:?}");
        assert_eq!(fx.world.portraits, [shown].into(), "{strings:?}");
    }
    // With no stack object the tokens stay (0x44c183 to 0x44c1a6), and
    // nothing is shown.
    for strings in [(0, 0, 4), (0, 0, 5)] {
        let mut fx = text_dialog(strings, Some(vec![(1, "Grim")]));
        assert_eq!(fx.run_with(LAMP, 4096, 0, &[], 1000), Ok(true));
        assert!(fx.world.portraits.is_empty());
    }
}

#[test]
fn an_object_name_that_may_hold_a_neighbor_token_fails_loudly() {
    // `$Object` is replaced by the stack object's catalog name (0x44c1ca,
    // 0x4573d9), and the `$Neighbor` pass after it reads the text with the
    // name in it: a name holding `$Neighbor`, or making it with the text
    // around it, shows the neighbor's type. The entry 0x4061e3 picks is not
    // ported, so every entry is tried; the names are not known when the
    // table has no entry in language 1, an entry is empty (which falls
    // back to the definition's own name, 0x457db4), or there is no table.
    // Where the names do not agree on the portrait, the dialog fails; names
    // not known fail too, since one may never end (0x4558a0).
    for (names, message, fault_at) in [
        (None, 3, "not known"),
        (Some(vec![(1, "Grim"), (2, "$Neighbor")]), 3, "0x4061e3"),
        (Some(vec![(1, "")]), 3, "not known"),
        (Some(vec![(2, "Grim")]), 3, "not known"),
        (Some(vec![(1, "Neighbor"), (2, "Grim")]), 5, "0x4061e3"),
        (None, 5, "not known"),
    ] {
        for strings in [(2, message, 0), (0, 0, message)] {
            let mut fx = text_dialog(strings, names.clone());
            let fault = fx
                .run_with(LAMP, 4096, SIM as i16, &[], 1000)
                .unwrap_err()
                .to_string();
            assert!(fault.contains(fault_at), "{names:?} {strings:?}: {fault}");
            assert!(fx.world.portraits.is_empty());
        }
        // Where the neighbor is found after the name, the name does not
        // matter to the portrait. A name not known may hold `$Object` past
        // its first character, which never ends.
        let mut fx = text_dialog((message, 0, 2), names.clone());
        let ran = fx.run_with(LAMP, 4096, SIM as i16, &[], 1000);
        if fault_at == "not known" {
            let fault = ran.unwrap_err().to_string();
            assert!(
                fault.contains("0x44c1ca") && fault.contains("not known"),
                "{names:?}: {fault}"
            );
        } else {
            assert_eq!(ran, Ok(true));
            assert_eq!(fx.world.portraits, [WIDGET].into(), "{names:?}");
        }
    }
    // Where every name gives the same, it is used: each holds `$Neighbor`,
    // or makes it with the `$` before the token, or none does.
    for (names, message, shown) in [
        (vec![(1, "Grim $Neighbor")], 3, WIDGET),
        (vec![(1, "Neighbor"), (2, "Neighbors")], 5, WIDGET),
        (vec![(1, "Grim"), (2, "Grimm")], 5, SIM_GUID),
    ] {
        let mut fx = text_dialog((0, 0, message), Some(names.clone()));
        assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
        assert_eq!(fx.world.portraits, [shown].into(), "{names:?}");
    }
    // A legacy catalog string table (CATS) in the object's file, which
    // 0x4061e3 checks first, leaves the name not known.
    let mut fx = text_dialog((0, 0, 3), Some(vec![(1, "Grim")]));
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .legacy_catalog_strings
        .insert(SIM_NAMES);
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x44c1ca"), "{fault}");
}

#[test]
fn neighbor_is_replaced_by_the_neighbors_name_and_may_never_end() {
    // `$Neighbor` (0x44c65f) is replaced by the name of the neighbor
    // record's type (0x4573d9, 0x44c6ba), and the search goes on from one
    // past it (0x44c6d7 to 0x44c6e8): a name that holds the token past its
    // first character never ends, in every mode and with no locals; names
    // not known may be one.
    for names in [Some(vec![(1, "my $Neighbor")]), None] {
        for mode in [0, 1] {
            let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
            set_message(&mut fx, "$Neighbor");
            name_widget(&mut fx, names.clone());
            in_mode(&mut fx, mode, 0);
            let fault = fx
                .run_with(LAMP, 4096, SIM as i16, &[], 1000)
                .unwrap_err()
                .to_string();
            assert!(fault.contains("0x44c6ba"), "{names:?} {mode}: {fault}");
            assert!(fx.world.portraits.is_empty());
        }
    }
    // One that holds it at its start is replaced once; with no neighbor
    // record the token stays.
    let mut fx = text_dialog((0, 0, 2), Some(vec![(1, "Grim")]));
    name_widget(&mut fx, Some(vec![(1, "$Neighbors")]));
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    let mut fx = text_dialog((0, 0, 2), Some(vec![(1, "Grim")]));
    name_widget(&mut fx, None);
    assert_eq!(fx.run_with(LAMP, 4096, 0, &[], 1000), Ok(true));
    assert!(fx.world.portraits.is_empty());
}

#[test]
fn a_token_of_a_pass_not_ported_fails_loudly() {
    // The formatter's passes that are not ported (0x44bf1a): `$FamilyAssets`
    // and `$Family` (0x44bf66, 0x44c034) before `$Me`'s; `$Local`, `$Attrib`,
    // `$NameAttrib`, `$NameLocal` and `$TokenNameLocal` between `$Object`'s
    // and `$NeighborLocal`'s after `$Local`'s (they read locals and the
    // stack object's attributes, and may fault, throw or add an inventory:
    // 0x44c2dd, 0x44c381, 0x44c57b, 0x42a8a1). A text that holds one of
    // their tokens when they run fails loudly, whatever the token's number,
    // the stack object, the mode, the locals or the object running. So does
    // one with a career token after `$Neighbor`'s (0x44c966) when the object
    // running is not a person, which the pass casts it to (0x44c9bc) and
    // reads (0x44ca27). `$CatalogLocal`, `$FameTitleLocal` and `$Grade`
    // after `$Neighbor`'s (0x44c6ea to 0x44c8c4) are not ported either:
    // what they write may hold their token again or make a career token
    // with the text around it (`$Grade` writes nothing for some numbers).
    let tokens = [
        ("$CatalogLocal:0", "`$CatalogLocal`"),
        ("$FameTitleLocal:0", "`$FameTitleLocal`"),
        ("$Grade", "`$Grade`"),
        ("$Jo$Grade:16b", "`$Grade`"),
        ("$FamilyAssets", "0x44c034"),
        ("$Family", "0x44c034"),
        ("$Attrib", "`$Attrib`"),
        ("$NameAttrib:0", "`$NameAttrib`"),
        ("$NameLocal:9", "`$NameLocal`"),
        ("$TokenNameLocal", "`$TokenNameLocal`"),
        ("$TokenNameLocal:0000000000000001", "`$TokenNameLocal`"),
        ("$Local:0 $Attrib", "`$Attrib`"),
        ("$Job", "0x44ca27"),
        ("$JobOffer:0:1", "0x44ca27"),
        ("$JobDesc:9", "0x44ca27"),
    ];
    for (text, said) in tokens {
        for (mode, stack_object) in [(0, SIM as i16), (0, 0), (1, SIM as i16), (1, 0)] {
            let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
            set_message(&mut fx, text);
            in_mode(&mut fx, mode, 0);
            let fault = fx
                .run_with(LAMP, 4096, stack_object, &[], 1000)
                .unwrap_err()
                .to_string();
            assert!(fault.contains(said), "{text} {mode}: {fault}");
            assert!(fx.world.portraits.is_empty(), "{text}");
        }
    }
    // In every icon mode (0x436219 to 0x436265), and in mode 4's operand 1
    // string (0x4362c0).
    for (mode, extra) in [(2, 0), (4, 0), (4, 2), (5, 0)] {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        let message = if extra == 0 { "$Job" } else { "Hello" };
        set_message(&mut fx, message);
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .strings
            .get_mut(&301)
            .unwrap()[1] = "$Job".to_owned();
        in_mode(&mut fx, mode, extra);
        let fault = fx
            .run_with(LAMP, 4096, SIM as i16, &[], 1000)
            .unwrap_err()
            .to_string();
        assert!(fault.contains("0x44ca27"), "mode {mode}: {fault}");
    }
    // A tree with no locals.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    set_message(&mut fx, "$Job $TokenNameLocal:0");
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8002,
            0,
            4,
            0,
            vec![ins(
                DIALOG,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 1, 0, 0, 0, 0, 1 | 1 << 1],
            )],
        ),
    );
    let fault = fx
        .run_with(LAMP, 4096, 0, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("`$TokenNameLocal`"), "{fault}");
    // A person running it: the inventories are as they were.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .strings
        .insert(301, vec!["$TokenNameLocal:0".to_owned()]);
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 1, 0, 0, 0, 0, 1],
        )]),
    );
    let inventories = fx.world.neighbors.inventories.clone();
    let fault = fx
        .run_with(SIM, 4096, 0, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("`$TokenNameLocal`"), "{fault}");
    assert_eq!(fx.world.neighbors.inventories, inventories);
    // The passes after `$Neighbor`'s that are not ported fail for a person
    // too.
    for text in ["$CatalogLocal:0", "$FameTitleLocal:0", "$Grade:16"] {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .strings
            .insert(301, vec![text.to_owned()]);
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![ins(
                DIALOG,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 1, 0, 0, 0, 0, 1 | 1 << 1],
            )]),
        );
        let fault = fx
            .run_with(SIM, 4096, 0, &[], 1000)
            .unwrap_err()
            .to_string();
        assert!(fault.contains("0x44c6ea to 0x44c8c4"), "{text}: {fault}");
    }
    // A name a ported pass writes is read by the passes after it: the
    // lamp's (`$Me`), the stack object's (`$Object`), the neighbor's
    // (`$Neighbor`).
    for (message, lamp, sim, widget, said) in [
        ("$Me", "my $Attrib", "Grim", "Widget", "`$Attrib`"),
        (
            "$Object",
            "Lamp",
            "Grim $NameLocal",
            "Widget",
            "`$NameLocal`",
        ),
        ("$Neighbor", "Lamp", "Grim", "a $Job", "0x44ca27"),
        ("$Neighbor", "Lamp", "Grim", "a $Grade", "`$Grade`"),
        (
            "$Me",
            "my $CatalogLocal:0",
            "Grim",
            "Widget",
            "`$CatalogLocal`",
        ),
        ("$Object", "Lamp", "Grim $Job", "Widget", "0x44ca27"),
    ] {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, sim)]));
        set_message(&mut fx, message);
        name_lamp(&mut fx, vec![(1, lamp)]);
        name_widget(&mut fx, Some(vec![(1, widget)]));
        in_mode(&mut fx, 1, 0);
        let fault = fx
            .run_with(LAMP, 4096, SIM as i16, &[], 1000)
            .unwrap_err()
            .to_string();
        assert!(fault.contains(said), "{message}: {fault}");
    }
    // Tokens a pass's name puts in after that pass has run are left: a
    // `$Family` from `$Me`, a `$Local` or `$Attrib` from `$Neighbor`.
    for (message, lamp, widget) in [
        ("$Me", "a $Family", "Widget"),
        ("$Neighbor", "Lamp", "a $Local:0 $Attrib"),
    ] {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        set_message(&mut fx, message);
        name_lamp(&mut fx, vec![(1, lamp)]);
        name_widget(&mut fx, Some(vec![(1, widget)]));
        assert_eq!(
            fx.run_with(LAMP, 4096, SIM as i16, &[], 1000),
            Ok(true),
            "{message}"
        );
    }
}

#[test]
fn the_passes_after_neighbors_are_presentation_for_a_person() {
    // `$TimeLocal` (0x44c8c4 to 0x44c961) writes text, in any mode and for
    // any object running; the career's (0x44c966) for a person, when the
    // texts they may write are known to hold no career token past their
    // first character. Not worked out.
    for text in ["$TimeLocal:0", "$TimeLocal:2 to $TimeLocal:3"] {
        for mode in [0, 1] {
            let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
            set_message(&mut fx, text);
            in_mode(&mut fx, mode, 0);
            assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
        }
    }
    let promoted = || {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .strings
            .insert(
                301,
                vec!["Promoted (to $Job:0:1)! $JobDesc:0:1 $JobOffer".to_owned()],
            );
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![ins(
                DIALOG,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 1, 0, 0, 0, 0, 1 | 1 << 1],
            )]),
        );
        fx
    };
    assert_eq!(promoted().run_with(SIM, 4096, 0, &[], 1000), Ok(true));
    // When they are not known to (`work.iff` not read, or a text of it that
    // does), a career pass may never end (0x44ca9f to 0x44cad3).
    let mut fx = promoted();
    fx.program.career_texts_end = false;
    let fault = fx
        .run_with(SIM, 4096, 0, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x44ca9f"), "{fault}");
}

#[test]
fn a_tokens_digits_up_to_a_byte_above_0x7f_fail_loudly() {
    // 0x44caf6 reads a `$Local` or `$NeighborLocal` token's `:` and digits
    // with a digit test handed each byte sign-extended (0x44cb70,
    // 0x636993): a byte above 0x7f where the digits stop is looked up below
    // the C runtime's table. Here a `«` (0xab in the table's code page).
    for text in [
        "$Local:\u{ab}",
        "$Local:12\u{ab}",
        "$Local:1:\u{ab}",
        "$Local:1:2\u{e9}",
        "$NeighborLocal:\u{ab}",
        "$NeighborLocal:$Local:\u{ab}",
    ] {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        set_message(&mut fx, text);
        in_mode(&mut fx, 1, 0);
        let fault = fx
            .run_with(LAMP, 4096, SIM as i16, &[], 1000)
            .unwrap_err()
            .to_string();
        assert!(fault.contains("0x44cb70"), "{text}: {fault}");
    }
    // Where the digits stop at an ASCII byte, or there is no `:`, the
    // test is not handed the byte.
    for text in ["$Local:12 \u{ab}", "$Local\u{ab}", "$NeighborLocal\u{ab}:"] {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        set_message(&mut fx, text);
        in_mode(&mut fx, 1, 0);
        assert_eq!(
            fx.run_with(LAMP, 4096, SIM as i16, &[], 1000),
            Ok(true),
            "{text}"
        );
    }
}

#[test]
fn a_dialogs_strings_are_read_in_the_order_the_loader_gives() {
    // A table with entries in more than one language is read in the order
    // the original's loader puts its first-language entries in (0x46ffcc
    // to 0x470069), which the library keeps apart.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    let library = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    let mut loaded = library.strings[&301].clone();
    loaded.swap(0, 1);
    library.loaded_strings.insert(301, Some(loaded));
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    // A file with no STR# 301 but a legacy CST 301, which the loader reads
    // instead (0x4702c7), fails loudly when the dialog names a string.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    let library = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    library.strings.remove(&301);
    library.legacy_strings.insert(301);
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x4702c7"), "{fault}");
    let mut fx = text_dialog((0, 0, 0), Some(vec![(1, "Grim")]));
    let library = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    library.strings.remove(&301);
    library.legacy_strings.insert(301);
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    // So does one whose table is only deleted chunks: the loader finds no
    // STR# 301 (0x41c542), whatever the chunks hold.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    let library = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    library.loaded_strings.insert(301, None);
    library.legacy_strings.insert(301);
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x4702c7"), "{fault}");
    // With no table at all, every string is its default: here no
    // portrait is found in the text.
    let mut fx = text_dialog((0, 0, 2), Some(vec![(1, "Grim")]));
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .loaded_strings
        .insert(301, None);
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    // A table whose entries have a language and none in US English is read
    // again for English in a way not ported (0x47006b to 0x4700d0).
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .english_fallback
        .insert(301);
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x47006b"), "{fault}");
}

#[test]
fn a_default_string_fails_loudly_unless_the_labels_hold_no_token() {
    // A string not named (here the button's label, operand 3 = 0) is its
    // default, a label from STR# 142 of `GameData/UIText.iff` (0x435fee,
    // 0x43600a, 0x436a21), which the formatter reads too.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    fx.program.labels_hold_no_token = false;
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x435fee"), "{fault}");
    // With every string named, no default is read.
    let mut fx = text_dialog((1, 1, 1), Some(vec![(1, "Grim")]));
    fx.program.labels_hold_no_token = false;
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    // An empty string is its default too (0x4369de).
    let mut fx = text_dialog((1, 1, 1), Some(vec![(1, "Grim")]));
    fx.program.labels_hold_no_token = false;
    let library = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    library.strings.get_mut(&301).unwrap()[0].clear();
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x435fee"), "{fault}");
}

#[test]
fn with_nothing_found_mode_0_shows_the_type_the_definition_stands_for() {
    // With no type found in the text, mode 0 shows the type of the
    // definition the stack object's stands for (0x436308 to 0x43634d,
    // 0x45728d, 0x43ae0a): its group's master's for a tile, here one of
    // the widget's type.
    let tile = |master_word: i16| {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        fx.words(SIM_FILE)[10] = 5;
        let mut master = definition(WIDGET, 2);
        master[10] = 5;
        master[11] = master_word;
        let library = fx.program.libraries.get_mut(&SIM_FILE).unwrap();
        library.definitions.insert(129, master);
        library.definition_order.push(129);
        fx
    };
    let mut fx = tile(-1);
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    // A tile whose group has no master leaves the place null: no portrait.
    let mut no_master = tile(0);
    assert_eq!(
        no_master.run_with(LAMP, 4096, SIM as i16, &[], 1000),
        Ok(true)
    );
    assert!(no_master.world.portraits.is_empty());
    // A type the text found wins over it.
    let mut fx = text_dialog((0, 0, 3), Some(vec![(1, "Grim")]));
    fx.words(SIM_FILE)[10] = 5;
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
}

#[test]
fn local_is_replaced_by_the_locals_value() {
    // `$Local` (0x44c1f2) is replaced, with its `:` and numbers, by the
    // value of the local its first number names, in decimal (0x46f941),
    // before `$NeighborLocal`'s pass reads the text. The number is its
    // first 15 digits (0x44cb52). A number that is not a local leaves the
    // token. Here local 0 is 3, local 1 is -7, local 2 is 0 and local 3
    // names neighbor record 2, of the widget's type, a person's: a
    // `$NeighborLocal` token naming it fails loudly (its name is not
    // ported).
    for (text, shown) in [
        ("$NeighborLocal:$Local:0000000000000003", None),
        ("$NeighborLocal:$Local:000000000000003", Some(WIDGET)),
        ("$NeighborLocal:$Local:1", Some(WIDGET)),
        ("$NeighborLocal:$Local:2:5", Some(WIDGET)),
        ("$NeighborLocal:$Local:0", None),
        ("$Local:9 $Local $Neighbor", Some(WIDGET)),
        ("Hello $Local:3", Some(SIM_GUID)),
    ] {
        let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
        set_message(&mut fx, text);
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 3, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, -7, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                assign(scope::LOCALS, 3, scope::LITERAL, SIM as i16, 4),
                ins(DIALOG, RETURN_TRUE, RETURN_FALSE, [0, 0, 1, 0, 0, 0, 0, 1]),
            ]),
        );
        let ran = fx.run_with(LAMP, 4096, SIM as i16, &[], 1000);
        match shown {
            Some(shown) => {
                assert_eq!(ran, Ok(true), "{text}");
                assert_eq!(fx.world.portraits, [shown].into(), "{text}");
            }
            None => {
                let fault = ran.unwrap_err().to_string();
                assert!(fault.contains("0x4e893e"), "{text}: {fault}");
            }
        }
    }
    // A local that starts as half of a pointer fails, as any read does.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "Grim")]));
    set_message(&mut fx, "$Local:1");
    fx.world.frame_storage.insert(
        LAMP,
        sims_sim::FrameStorage {
            unknown: [(0, 1)].into(),
            ..Default::default()
        },
    );
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains("0x44c240") && fault.contains("half of a pointer"),
        "{fault}"
    );
}

#[test]
fn me_is_replaced_by_the_running_objects_name_before_object_is() {
    // `$Me` (0x44c0fc) is replaced by the name of the object running the
    // tree (the formatter's `this`, 0x436200), here the lamp, before
    // `$Object`'s pass: a name holding `$Object` or `$Neighbor` is found
    // by the passes after it. The label (string 2, `$Neighbor`) comes
    // before the message.
    let with_lamp_names = |names: Option<Vec<(u8, &str)>>| {
        let mut fx = text_dialog((2, 0, 1), Some(vec![(1, "Grim")]));
        set_message(&mut fx, "$Me is here");
        fx.words(LAMP_FILE)[41] = LAMP_NAMES as i16;
        if let Some(names) = names {
            name(&mut fx, LAMP_FILE, LAMP_NAMES, names);
        }
        fx
    };
    for (names, shown) in [
        (vec![(1, "Lamp"), (2, "Lampe")], WIDGET),
        (vec![(1, "the $Object")], SIM_GUID),
        (vec![(1, "the $Object $Neighbor")], WIDGET),
    ] {
        let mut fx = with_lamp_names(Some(names.clone()));
        assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
        assert_eq!(fx.world.portraits, [shown].into(), "{names:?}");
    }
    // Each token is replaced from one past the place before (0x44c150):
    // a name `x$` before `Me` makes `$Me` again, and that one's name then
    // makes `$Object`.
    let mut fx = with_lamp_names(Some(vec![(1, "x$")]));
    set_message(&mut fx, "$MeMeObject");
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    // A name that holds `$Me` at its start is replaced once: the search
    // goes on past its first character.
    let mut fx = with_lamp_names(Some(vec![(1, "$Me $Object")]));
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    // Names that disagree fail where they decide the portrait. A name that
    // holds `$Me` past its first character puts it back past each place
    // the search goes on from, so the pass never ends; names not known may
    // be one. Both fail whatever the portrait.
    for (names, fault_at) in [
        (Some(vec![(1, "Lamp"), (2, "$Object")]), "0x4061e3"),
        (Some(vec![(1, "my $Me")]), "never ends"),
        (Some(vec![(1, "Lamp"), (2, "$$Me")]), "never ends"),
        (None, "not known"),
    ] {
        let mut fx = with_lamp_names(names.clone());
        let fault = fx
            .run_with(LAMP, 4096, SIM as i16, &[], 1000)
            .unwrap_err()
            .to_string();
        assert!(fault.contains(fault_at), "{names:?}: {fault}");
        assert!(fx.world.portraits.is_empty());
    }
    // Whatever the portrait: here `$Neighbor` comes last.
    let mut fx = with_lamp_names(Some(vec![(1, "my $Me")]));
    set_message(&mut fx, "$Me $Neighbor");
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains("never ends") && fault.contains("0x44c13e"),
        "{fault}"
    );
}

#[test]
fn object_is_replaced_again_from_one_past_each_place() {
    // `$Object`'s pass, as `$Me`'s, searches again from one past each place
    // it replaced (0x44c1e3): a name `x$` before `Object` makes `$Object`
    // again, and its name then makes `$Neighbor`.
    let mut fx = text_dialog((0, 0, 1), Some(vec![(1, "x$")]));
    set_message(&mut fx, "$ObjectObjectNeighbor");
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    // A name that holds `$Object` at its start is replaced once.
    let mut fx = text_dialog((2, 0, 3), Some(vec![(1, "$Objects")]));
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    // One that holds it past its first character never ends, even where
    // the portrait is certain: the person runs the dialog and the stack
    // object has no neighbor record of a type nobody has.
    for strings in [(2, 0, 3), (0, 0, 3), (3, 0, 2)] {
        let mut fx = text_dialog(strings, Some(vec![(1, "my $Object")]));
        let fault = fx
            .run_with(LAMP, 4096, SIM as i16, &[], 1000)
            .unwrap_err()
            .to_string();
        assert!(
            fault.contains("never ends") && fault.contains("0x44c1ca"),
            "{strings:?}: {fault}"
        );
        assert!(fx.world.portraits.is_empty());
    }
}

#[test]
fn a_neighbor_local_named_in_a_dialogs_text_fails_loudly() {
    // The formatter's `$NeighborLocal` pass (0x44c590 to 0x44c65a) comes
    // before the `$Neighbor` pass, which finds `$Neighbor` anywhere in the
    // text as it is left. A token whose local names a record of a person
    // type is replaced by that type's name from the interface's table,
    // which is not ported, and fails loudly. One whose local names no
    // record, or is past the tree's locals, stays, and `$Neighbor` is found
    // in it. A record of a type not in the person types' table ends the
    // pass (0x44c616), leaving the tokens after it too. The number is its
    // first 15 digits (0x44cb52, 0x46f78d), wrapping (0x636029).
    let strings = [
        "Hi $NeighborLocal:0",
        "$NeighborLocal:9",
        "$NeighborLocal:1 $NeighborLocal:0",
        "$NeighborLocal:0 and $Neighbor",
        "$NeighborLocal:4294967297",
        "$NeighborLocal:4294967296",
        "$NeighborLocal:0000000000000001",
        "$NeighborLocal:000000000000001",
    ];
    for (message, locals, shown) in [
        (1, [0, 0], Some(WIDGET)),
        (2, [-1, -1], Some(WIDGET)),
        (3, [-1, -2], Some(WIDGET)),
        (4, [0, 0], Some(WIDGET)),
        (4, [-1, 0], None),
        (5, [-1, 0], Some(WIDGET)),
        (8, [-1, 0], Some(WIDGET)),
        (1, [-1, 0], None),
        (3, [-1, -1], None),
        (6, [-1, 0], None),
        (7, [-1, 0], None),
        (8, [0, -1], None),
    ] {
        // The person (2) is the stack object, and neighbor record 2 is of
        // the widget's type, with nobody of it on the lot. Local `n` holds
        // `locals[n]`, where -1 is the widget's record and -2 a record of
        // the lamp's type, which is not a person.
        let (mut fx, _, record) = visitors();
        let neighbor = fx.world.neighbors.slots[record as usize - 1].clone();
        let mut lamp = neighbor.clone().unwrap();
        lamp.guid = LAMP_GUID;
        let slots = &mut fx.world.neighbors.slots;
        slots.resize(slots.len().max(2), None);
        slots[usize::from(SIM) - 1] = neighbor;
        slots.push(Some(lamp));
        let lamp_record = slots.len() as i16;
        fx.world.people = vec![SIM];
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .strings
            .insert(301, strings.map(str::to_owned).to_vec());
        name_widget(&mut fx, Some(vec![(1, "Widget")]));
        let widget_file = *fx
            .program
            .libraries
            .iter()
            .find(|(_, library)| library.catalog_strings.contains_key(&WIDGET_NAMES))
            .unwrap()
            .0;
        fx.words(widget_file)[41] = WIDGET_NAMES as i16;
        let local = |value: i16| match value {
            -1 => record,
            -2 => lamp_record,
            v => v,
        };
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, local(locals[0]), 1),
                assign(scope::LOCALS, 1, scope::LITERAL, local(locals[1]), 2),
                ins(
                    DIALOG,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [0, 0, message, 0, 0, 0, 0, 1],
                ),
            ]),
        );
        let ran = fx.run_with(LAMP, 4096, SIM as i16, &[], 1000);
        match shown {
            Some(shown) => {
                assert_eq!(ran, Ok(true), "{message} {locals:?}");
                assert_eq!(fx.world.portraits, [shown].into(), "{message} {locals:?}");
            }
            None => {
                let fault = ran.unwrap_err().to_string();
                assert!(fault.contains("0x4e893e"), "{message} {locals:?}: {fault}");
                assert!(fx.world.portraits.is_empty());
            }
        }
    }
}

#[test]
fn a_neighbor_local_the_original_left_unknown_fails_loudly() {
    // The pass reads the local (0x44c5f4) as any read of a local does: one
    // that starts as half of a pointer left in the frame storage fails.
    let (mut fx, _, record) = visitors();
    let neighbor = fx.world.neighbors.slots[record as usize - 1].clone();
    let slots = &mut fx.world.neighbors.slots;
    slots.resize(slots.len().max(2), None);
    slots[usize::from(SIM) - 1] = neighbor;
    fx.world.people = vec![SIM];
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .strings
        .insert(301, vec!["$NeighborLocal:1".to_owned()]);
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 1, 0, 0, 0, 0, 1],
        )]),
    );
    fx.world.frame_storage.insert(
        LAMP,
        sims_sim::FrameStorage {
            unknown: [(0, 1)].into(),
            ..Default::default()
        },
    );
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains("0x44c5f4") && fault.contains("half of a pointer"),
        "{fault}"
    );
}
