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
    world.dirty_portraits.clear();
    world.saved_portraits.clear();
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
    // An eligible off-lot household member proceeds to 0x4e9652 (matching
    // native 0x4363d1 -> 0x4e92a8 -> 0x4e9652) and builds its portrait.
    let (mut fx, _, record) = visitors();
    fx.world.globals[9] = 7;
    fx.world.families.push(sims_sim::neighbors::Family {
        id: 7,
        house: 0,
        members: vec![WIDGET],
        friends: 0,
        stale: false,
    });
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert!(fx.world.portraits.contains(&WIDGET));

    // When the same off-lot household member has a clean slot and BMP_ 2002,
    // 0x4e92e5 → 0x4e9338 loads the bitmap directly with zero builds.
    let (mut fx, widget_file, record) = visitors();
    fx.world.globals[9] = 7;
    fx.world.families.push(sims_sim::neighbors::Family {
        id: 7,
        house: 0,
        members: vec![WIDGET],
        friends: 0,
        stale: false,
    });
    fx.program
        .libraries
        .get_mut(&widget_file)
        .unwrap()
        .has_portrait_bitmap = true;
    let before = as_with_no_portrait(&fx, record);
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.dirty_portraits.is_empty());
    assert_eq!(rest(&fx), before);
}

#[test]
fn a_runtime_made_person_type_is_listed_in_the_portrait_table() {
    // 0x4265fd calls 0x4e8a3a once the table is initialized ([0x6f26a4] = 1 at
    // 0x4e8a32), appending newly created person types at 0x4e8add. Cloning
    // copies every resource from the template (0x472356..0x472519), including
    // BMP_ 2002 when present (as on TemplateCat.iff and TemplateDog.iff), into
    // a new writable User%05d.iff file.
    const MADE_GUID: u32 = 0x5555_0001;
    let attach_made =
        |fx: &mut Fixture, strings: std::collections::BTreeMap<usize, String>| -> i16 {
            fx.world.characters.made.insert(
                MADE_GUID,
                sims_sim::characters::Character {
                    template: WIDGET,
                    file: 3,
                    strings,
                },
            );
            fx.world
                .neighbors
                .slots
                .push(Some(sims_sim::neighbors::Neighbor {
                    id: (fx.world.neighbors.slots.len() + 1) as i16,
                    guid: MADE_GUID,
                    revision: 7,
                    data: vec![12; 256],
                    relationships: Vec::new(),
                    friends: 0,
                    stale: false,
                }));
            fx.world.neighbors.slots.len() as i16
        };

    // 1. Template with BMP_ 2002 (e.g. TemplateCat/TemplateDog): the clone
    //    inherits BMP_ 2002, so a clean off-lot dialog loads directly with
    //    zero builds even on a blocked lot (and is not read-only even if the
    //    template file was read-only).
    let (mut fx_bmp, widget_file, _) = visitors();
    fx_bmp.world.size = 2;
    let template = fx_bmp.program.libraries.get_mut(&widget_file).unwrap();
    template.has_portrait_bitmap = true;
    template.writable = false;
    let made_record = attach_made(&mut fx_bmp, Default::default());
    let before = as_with_no_portrait(&fx_bmp, made_record);
    assert_eq!(inform(&mut fx_bmp, 2, made_record), Ok(true));
    assert_eq!(fx_bmp.world.portraits, [MADE_GUID].into());
    assert!(fx_bmp.world.saved_portraits.is_empty());
    assert_eq!(rest(&fx_bmp), before);

    // 2. Template without BMP_ 2002 (e.g. TemplatePerson): the clone's own
    //    User%05d.iff is writable even if the template was read-only, so an
    //    off-lot dialog builds once and saves BMP_ 2002 at 0x4e938c → 0x4e9000.
    let (mut fx, widget_file, _) = visitors();
    fx.program.libraries.get_mut(&widget_file).unwrap().writable = false;
    let made_record = attach_made(&mut fx, Default::default());
    let before = as_with_no_portrait(&fx, made_record);
    assert_eq!(inform(&mut fx, 2, made_record), Ok(true));
    assert_eq!(fx.world.portraits, [MADE_GUID].into());
    assert_eq!(fx.world.saved_portraits, [MADE_GUID].into());
    assert_ne!(rest(&fx), before);

    // 3. On-lot: builds directly with zero temporary persons and records
    //    MADE_GUID (not its template GUID).
    let (mut fx_on, _, _) = visitors();
    fx_on.world.characters.made.insert(
        MADE_GUID,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 3,
            strings: Default::default(),
        },
    );
    let mut person = object(40, MADE_GUID);
    person.person = Some(person_data());
    fx_on.world.objects.insert(40, person);
    fx_on.world.people.push(40);
    let before_on = as_with_no_portrait(&fx_on, 40);
    assert_eq!(inform(&mut fx_on, 0, 40), Ok(true));
    assert_eq!(fx_on.world.portraits, [MADE_GUID].into());
    assert_eq!(fx_on.world.saved_portraits, [MADE_GUID].into());
    assert_eq!(rest(&fx_on), before_on);
}

fn person_data() -> sims_formats::saved_objects::Person {
    let interaction = sims_formats::saved_objects::Interaction {
        join: 0,
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: 0,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    };
    sims_formats::saved_objects::Person {
        animation_events: 0,
        engaged: 0,
        appearance: Default::default(),
        accessories: Vec::new(),
        animations: Default::default(),
        routing_state: 0,
        floats: [0.0; 9],
        previous_motives: [0.0; 16],
        motives: [0.0; 16],
        data: vec![0; 256],
        routing_frames: 0,
        active: interaction.clone(),
        last: interaction,
        queue: Vec::new(),
        object_uses: Vec::new(),
        motive_deltas: Vec::new(),
    }
}

#[test]
fn a_portrait_whose_person_runs_code_when_put_down_fails_loudly() {
    // The person made is put on a tile (0x45e4e9, 0x433340), which asks
    // it its function 9 (0x433580) on placement and function 10 (0x433148)
    // before removal. When a tile-independent PLACED callback runs, its
    // side effects are kept; when a callback fails (missing tree 4200) or
    // depends on which tile an unknown view centre picks, the build fails
    // loudly and leaves no temporary person behind.
    let (mut fx, file, record) = visitors();
    fx.add(
        file,
        4200,
        tree(vec![assign(
            scope::GLOBAL,
            36,
            scope::LITERAL,
            55,
            RETURN_TRUE,
        )]),
    );
    let library = fx.program.libraries.get_mut(&file).unwrap();
    for table in library.callbacks.values_mut() {
        table.resize(
            27,
            sims_formats::execution::ObjectFunction {
                condition: 0,
                action: 0,
            },
        );
        table[9].action = 4200;
    }
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert_eq!(fx.world.globals[36], 55);
    assert_eq!(fx.world.portraits, [WIDGET].into());

    // A PLACED callback whose effect depends on the room (writing `my room`
    // into global 36) fails loudly when no view centre is given across two
    // rooms, and leaves no temporary person behind.
    let (mut fx, file, record) = visitors();
    indoors_and_out(&mut fx);
    let objects: Vec<u16> = fx.world.objects.keys().copied().collect();
    fx.add(
        file,
        4200,
        tree(vec![assign(
            scope::GLOBAL,
            36,
            scope::MY_DATA,
            29,
            RETURN_TRUE,
        )]),
    );
    let library = fx.program.libraries.get_mut(&file).unwrap();
    for table in library.callbacks.values_mut() {
        table.resize(
            27,
            sims_formats::execution::ObjectFunction {
                condition: 0,
                action: 0,
            },
        );
        table[9].action = 4200;
    }
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("0x4e96dd"), "{fault}");
    assert!(fx.world.portraits.is_empty());
    assert_eq!(
        fx.world.objects.keys().copied().collect::<Vec<_>>(),
        objects
    );
}

#[test]
fn a_portrait_whose_put_depends_on_its_tile_fails_loudly() {
    // Across two rooms with no view centre given, data word 40 bit 0x400
    // (`0x433340`) and data word 7 (`0x433ed8`) mark whichever room the
    // unknown camera centre selects, so the build fails loudly.
    for (word, value) in [(40, 0x400), (7, 1)] {
        let (mut fx, file, record) = visitors();
        indoors_and_out(&mut fx);
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.add(
            file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
                assign(scope::MY_DATA, word, scope::LITERAL, value, RETURN_TRUE),
            ]),
        );
        let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
        assert!(fault.contains("0x4e96dd"), "{word}: {fault}");
        assert!(fx.world.portraits.is_empty());
    }
    // A person of kind 0 or 1 marks the indoor room it lands in as
    // occupied (0x45e43d), which its removal leaves: on `visitors()`, which
    // has both room 0 (outside) and room 1 (inside), leaving `view_centre`
    // unset fails loudly when landing in room 1 would flip `occupied` while
    // landing in room 0 would not; passing `view_centre = Some([2, 2])`
    // (inside room 1) succeeds natively and sets `occupied = true`.
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
            assert!(fault.contains("0x4e96dd"), "{kind}: {fault}");
            assert_eq!(inform_from(&mut fx, 2, record, Some([2, 2])), Ok(true));
            assert!(fx.world.rooms.rooms[&1].occupied);
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
    // No tile takes it from ANY start: the original removes the temporary
    // person unput (0x4e9706), keeps its Init/RNG side effects, leaves the
    // portrait uncached, and completes the dialog nonfatally.
    let (mut fx, _, record) = visitors();
    fx.world.size = 2;
    let before_rng = fx.world.crt_rng.state();
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert!(fx.world.portraits.is_empty());
    assert_ne!(fx.world.crt_rng.state(), before_rng);
    // When no view centre is given and SOME starts find a tile while others
    // do not, placement still fails loudly because the outcome depends on the
    // unknown camera start: put LAMP on (1,1) in its own isolated room 99.
    let (mut fx, _, record) = visitors();
    fx.world.rooms.map[0][64 + 1] = 99;
    fx.world.objects.get_mut(&LAMP).unwrap().position = [1, 1, 1];
    fx.world.objects.get_mut(&LAMP).unwrap().data[29] = 99;
    fx.world.arrays.get_mut(&3).unwrap().data[(64 + 1) * 2..(64 + 1) * 2 + 2]
        .copy_from_slice(&(LAMP as i16).to_le_bytes());
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
    // With no centre, some start ends indoors and others outdoors.
    let (mut fx, _, record) = visitors();
    rooms(&mut fx);
    let fault = inform_from(&mut fx, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x4e96dd"), "{fault}");
    // From an in-bounds centre outdoors the search ends outdoors, in the
    // start's room, and the lot ends the same. An out-of-bounds pick like
    // [0, 0] or [2, 60] falls back to the lot midpoint [3, 3] (0x434891),
    // which is indoors and marks room 1 occupied natively.
    let mut ends = Vec::new();
    for centre in [[1, 1], [2, 4]] {
        let (mut fx, _, record) = visitors();
        rooms(&mut fx);
        assert_eq!(inform_from(&mut fx, 2, record, Some(centre)), Ok(true));
        assert_eq!(fx.world.portraits, [WIDGET].into());
        ends.push(hash_without_frames(&fx.world));
    }
    assert!(ends.windows(2).all(|w| w[0] == w[1]));
    for centre in [[0, 0], [2, 60], [4, 2]] {
        let (mut fx, _, record) = visitors();
        rooms(&mut fx);
        assert_eq!(
            inform_from(&mut fx, 2, record, Some(centre)),
            Ok(true),
            "{centre:?}"
        );
        assert_eq!(fx.world.portraits, [WIDGET].into(), "{centre:?}");
        assert!(fx.world.rooms.rooms[&1].occupied, "{centre:?}");
    }
    // From a centre no tile is found from, the original removes the temporary
    // person unput (0x4e9706), leaves the portrait uncached, keeps Init/RNG
    // side effects, and completes the dialog nonfatally.
    let (mut fx, _, record) = visitors();
    fx.world.size = 2;
    let before_rng = fx.world.crt_rng.state();
    assert_eq!(inform_from(&mut fx, 2, record, Some([7, 7])), Ok(true));
    assert!(fx.world.portraits.is_empty());
    assert_ne!(fx.world.crt_rng.state(), before_rng);
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
    // What it does is read after the search: with a view centre given,
    // `put_person` runs natively and marks the room via `move_tree` (word 7)
    // and `put_one` (word 40 bit 0x400).
    for (word, value) in [(7, 1), (40, 0x400)] {
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
        assert_eq!(inform_from(&mut fx, 2, record, Some([3, 4])), Ok(true));
        assert_eq!(fx.world.portraits, [WIDGET].into());
    }
}

#[test]
fn a_portrait_put_that_takes_the_mark_off_the_room_it_was_last_seen_in_fails_loudly() {
    // The person's Init names room 1 in its data word 29, and its first
    // tick, which the factory runs, notes that as the room it was last
    // seen in (0x45e43d). Put outdoors with a view centre, `put_person` runs
    // `note_room` natively and clears room 1's occupied mark when nobody else
    // was last seen there (0x46886c, 0x468802). With no view centre across
    // indoors and outdoors, the outcome depends on the unknown centre and
    // fails loudly.
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
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert!(!fx.world.rooms.rooms[&1].occupied);
    // With no centre, indoors keeps room 1 occupied while outdoors clears it.
    let (mut fx, record) = setup(false);
    let rooms = fx.world.rooms.clone();
    let fault = inform_from(&mut fx, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x4e96dd"), "{fault}");
    assert_eq!(fx.world.rooms, rooms);
    // Someone else last seen in room 1 keeps its mark.
    let (mut fx, record) = setup(true);
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert!(fx.world.rooms.rooms[&1].occupied);
}

#[test]
fn a_portrait_put_on_a_lot_with_no_indicators_fails_loudly() {
    // With a view centre (or invariant across tiles), `put_person` creates
    // the lot's indicators natively (`0x445e4a`) while the temporary person
    // stands on its tile.
    let (mut fx, _, record) = visitors();
    for guid in [0x2f72_6fff, 0x0baa_71c5] {
        let file = fx.add_type(guid, 4, 4096, 4097);
        fx.add(
            file,
            4096,
            tree(vec![ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
        );
        fx.add(
            file,
            4097,
            tree(vec![ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
        );
    }
    assert_ne!(fx.world.indicators[0], 0);
    fx.world.indicators = [0, 0];
    assert_eq!(inform_from(&mut fx, 2, record, Some([2, 2])), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert_ne!(fx.world.indicators[0], 0);
    assert_ne!(fx.world.indicators[1], 0);
}

#[test]
fn a_portrait_build_that_fails_leaves_the_machines_cells_as_they_were() {
    // The person's Init plays a sound, and the put then fails across two
    // rooms when no view centre is given (data word 40 bit 0x400, 0x433340):
    // the sound is not kept, as the person is not. Without the bit the build
    // is kept, the sound with it.
    for bit in [0x400, 0] {
        let (mut fx, file, record) = visitors();
        indoors_and_out(&mut fx);
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
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
        let ran = inform_hearing(&mut fx, 2, record, None, &heard);
        let heard = heard.into_inner();
        if bit != 0 {
            let fault = ran.unwrap_err().to_string();
            assert!(fault.contains("0x4e96dd"), "{fault}");
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
    // 0x4573d9, 0x4061e3), and the `$Neighbor` pass after it reads the text
    // with the name in it: a name holding `$Neighbor`, or making it with
    // the text around it, shows the neighbor's type. `0x4061e3` loads US
    // English via `0x470ca7(0, 0)` and reads slot 1 (`0x471411(1, -1)`):
    // the name is not known when the table has no entry in language 1, slot
    // 1 is empty (which falls back to every language for a person or the
    // definition's own name, 0x457db4), or there is no table. Names not
    // known fail loudly, since one may never end (0x4558a0).
    for (names, message) in [
        (None, 3),
        (Some(vec![(1, "")]), 3),
        (Some(vec![(2, "Grim")]), 3),
        (None, 5),
    ] {
        for strings in [(2, message, 0), (0, 0, message)] {
            let mut fx = text_dialog(strings, names.clone());
            let fault = fx
                .run_with(LAMP, 4096, SIM as i16, &[], 1000)
                .unwrap_err()
                .to_string();
            assert!(
                fault.contains("not known"),
                "{names:?} {strings:?}: {fault}"
            );
            assert!(fx.world.portraits.is_empty());
        }
        // Even where the neighbor is found after the name, a name not known
        // may hold `$Object` past its first character, which never ends.
        let mut fx = text_dialog((message, 0, 2), names.clone());
        let fault = fx
            .run_with(LAMP, 4096, SIM as i16, &[], 1000)
            .unwrap_err()
            .to_string();
        assert!(
            fault.contains("0x44c1ca") && fault.contains("not known"),
            "{names:?}: {fault}"
        );
    }
    // `0x4061e3` picks slot 1 in US English (`0x470ca7(0, 0)` + `0x471343`):
    // slot 2 (the description) and other languages are ignored, and a later
    // US English run overwrites slot 1.
    for (names, message, shown) in [
        (vec![(1, "Grim $Neighbor")], 3, WIDGET),
        (vec![(1, "Grim"), (1, "$Neighbor")], 3, SIM_GUID),
        (vec![(1, "Grim"), (2, "$Neighbor")], 3, SIM_GUID),
        (vec![(1, "Grim"), (2, "Other"), (1, "$Neighbor")], 3, WIDGET),
        (vec![(1, "Neighbor"), (2, "Grim")], 5, WIDGET),
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
        .unported_strings
        .insert(
            301,
            "has entries with a language and none in US English (0x47006b)",
        );
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
        (vec![(1, "Lamp"), (2, "$Object")], WIDGET),
        (vec![(1, "Lamp"), (2, "$$Me")], WIDGET),
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
    // A name that holds `$Me` past its first character puts it back past
    // each place the search goes on from, so the pass never ends; names
    // not known may be one. Both fail whatever the portrait.
    for (names, fault_at) in [
        (Some(vec![(1, "my $Me")]), "never ends"),
        (
            Some(vec![(1, "Lamp"), (2, "Other"), (1, "$$Me")]),
            "never ends",
        ),
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

#[test]
fn a_portrait_view_centre_pick_is_bounds_checked_and_unrotated() {
    // On a 6-tile lot, tiles with x < 3 are outdoors (room 2) and x >= 3
    // are indoors (room 1). At view rotation 1, lot tile (x, y) rotates to
    // (63 - y, x) so the rotated interior is [59..=62, 1..=4].
    // View-rotated pick [60, 2] un-rotates to lot tile (2, 3) (outdoors -> Ok).
    // View-rotated pick [60, 4] un-rotates to lot tile (4, 3) (indoors -> fails).
    // Out-of-bounds pick [2, 2] falls back to rotated midpoint (61, 3), which
    // un-rotates to lot tile (3, 2) (indoors -> fails).
    for (rot, pick, should_pass) in [
        (1u8, [60i8, 2], true),
        (1, [60, 4], false),
        (1, [2, 2], false),
        (3, [2, 60], false), // un-rotates to (3, 2), indoors
        (3, [2, 61], true),  // un-rotates to (2, 2), outdoors
        (3, [2, 2], true),   // out-of-bounds fallback (3, 61) -> un-rotates to (2, 3), outdoors
    ] {
        let (mut fx, _, record) = visitors();
        indoors_and_out(&mut fx);
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                DIALOG,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
            )]),
        );
        let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.view_rotation = rot;
        interpreter.view_centre = Some(pick);
        let res = interpreter.run_tree(&mut fx.world, LAMP, 4096, record, &[], 1000);
        assert_eq!(res, Ok(true), "rot {rot} pick {pick:?}");
        assert_eq!(
            fx.world.rooms.rooms[&1].occupied, !should_pass,
            "rot {rot} pick {pick:?}"
        );
    }
}

#[test]
fn a_clean_portrait_with_bmp_2002_loads_without_creating_a_person() {
    // 0x4e92e5 → 0x4e9338: a clean, empty slot with BMP_ 2002 loads the
    // bitmap straight into slot 1 and never calls 0x4e8c5d (0 builds: no
    // temporary person, no Init sounds, no skeleton read, no RNG draws).
    let (mut fx, widget_file, record) = visitors();
    fx.program
        .libraries
        .get_mut(&widget_file)
        .unwrap()
        .sounds
        .insert(5, "init_ping".into());
    fx.add(
        widget_file,
        4096,
        tree(vec![ins(23, RETURN_TRUE, ERROR, [5, 0, 0, 0, 0, 0, 0, 0])]),
    );
    fx.program
        .libraries
        .get_mut(&widget_file)
        .unwrap()
        .has_portrait_bitmap = true;

    let before = as_with_no_portrait(&fx, record);
    let sounds = std::cell::RefCell::new(Vec::new());
    assert_eq!(
        inform_hearing(&mut fx, 2, record, Some([1, 1]), &sounds),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.dirty_portraits.is_empty());
    assert!(
        !fx.world.skeleton_files.contains("widget.bcf"),
        "clean slot with BMP_ 2002 never creates a temporary person"
    );
    assert!(sounds.borrow().is_empty(), "{:?}", sounds.borrow());
    assert_eq!(rest(&fx), before);

    // Writing person data words 68, 74, or 84 calls 0x45810c(1) on [esi+0x11c]
    // (0x4539c7) — the running object's definition (`me`), even in scope 19
    // (Stack Person Data) when targeting another person.
    for word in [68i16, 74, 84] {
        // 1. SIM writing My Person Data dirties SIM_GUID.
        let mut fx = Fixture::new();
        fx.world.people = vec![SIM];
        fx.world.portraits.insert(SIM_GUID);
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![assign(
                scope::MY_PERSON,
                word,
                scope::LITERAL,
                0,
                RETURN_TRUE,
            )]),
        );
        assert_eq!(fx.run_with(SIM, 4096, 0, &[], 1000), Ok(true));
        assert!(
            !fx.world.portraits.contains(&SIM_GUID),
            "SIM writing My Person Data[{word}] releases cached portrait"
        );
        assert!(
            fx.world.dirty_portraits.contains(&SIM_GUID),
            "SIM writing My Person Data[{word}] marks slot 1 dirty"
        );

        // 2. A non-person (LAMP) writing Stack Person Data targeting SIM does
        //    NOT dirty SIM_GUID because [esi+0x11c] is LAMP's definition.
        let mut fx = Fixture::new();
        fx.world.people = vec![SIM];
        fx.world.portraits.insert(SIM_GUID);
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![assign(
                scope::STACK_PERSON,
                word,
                scope::LITERAL,
                0,
                RETURN_TRUE,
            )]),
        );
        assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(true));
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "LAMP writing Stack Person Data[{word}] keeps SIM's cached portrait"
        );
        assert!(
            fx.world.dirty_portraits.is_empty(),
            "LAMP writing Stack Person Data[{word}] does not dirty SIM"
        );

        // 3. SIM writing Stack Person Data targeting another person (WIDGET)
        //    dirties SIM_GUID ([esi+0x11c]), NOT WIDGET.
        let (mut fx, _, _) = visitors();
        let mut other = object(40, WIDGET);
        other.person = Some(person_data());
        fx.world.objects.insert(40, other);
        fx.world.people.push(40);
        fx.world.portraits.insert(SIM_GUID);
        fx.world.portraits.insert(WIDGET);
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![assign(
                scope::STACK_PERSON,
                word,
                scope::LITERAL,
                0,
                RETURN_TRUE,
            )]),
        );
        assert_eq!(fx.run_with(SIM, 4096, 40, &[], 1000), Ok(true));
        assert!(
            !fx.world.portraits.contains(&SIM_GUID) && fx.world.dirty_portraits.contains(&SIM_GUID),
            "SIM writing Stack Person Data[{word}] on WIDGET dirties SIM"
        );
        assert!(
            fx.world.portraits.contains(&WIDGET) && !fx.world.dirty_portraits.contains(&WIDGET),
            "SIM writing Stack Person Data[{word}] on WIDGET leaves WIDGET clean"
        );
    }
}

#[test]
fn a_dirty_portrait_with_bmp_2002_removes_unput_when_no_tile_is_found() {
    // When slot 1 is dirty (0x4e92e5), no tile is found (0x4e9706), and the
    // type has BMP_ 2002 (0x4e9260), the original runs 0x4e92f2 once, removes
    // the person unput (skipping virtual function 0x88 and placed_from_outside),
    // loads BMP_ 2002 into slot 1 (clearing dirty), and skips 0x4e938c.
    let (mut fx, widget_file, record) = visitors();
    fx.world.size = 2;
    fx.world.dirty_portraits.insert(WIDGET);
    // Even without indicators and with function 9 set to trap, the unput path
    // never calls virtual function 0x88, so it succeeds when BMP_ 2002 is present.
    fx.world.indicators = [0, 0];
    crate::visits::function(&mut fx, widget_file, 9, 4098);
    fx.add(
        widget_file,
        4098,
        tree(vec![ins(254, RETURN_TRUE, ERROR, [0; 8])]),
    );
    fx.program
        .libraries
        .get_mut(&widget_file)
        .unwrap()
        .sounds
        .insert(5, "init_ping".into());
    fx.add(
        widget_file,
        4096,
        tree(vec![ins(23, RETURN_TRUE, ERROR, [5, 0, 0, 0, 0, 0, 0, 0])]),
    );
    fx.program
        .libraries
        .get_mut(&widget_file)
        .unwrap()
        .has_portrait_bitmap = true;

    let people_before = fx.world.people.clone();
    let objects_before = fx.world.objects.len();
    let sounds = std::cell::RefCell::new(Vec::new());
    assert_eq!(
        inform_hearing(&mut fx, 2, record, Some([1, 1]), &sounds),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.dirty_portraits.is_empty());
    assert_eq!(fx.world.people, people_before);
    assert_eq!(fx.world.objects.len(), objects_before);
    let pings = || {
        sounds
            .borrow()
            .iter()
            .filter(|e| matches!(e, sims_vm::SoundEvent::Play { name, .. } if name == "init_ping"))
            .count()
    };
    // One build (create_person + delete's reset each run Init once = 2 pings;
    // a second build at 0x4e938c would have produced 4).
    assert_eq!(pings(), 2);
    // A second dialog hits World::portraits and does not rebuild.
    assert_eq!(
        inform_hearing(&mut fx, 2, record, Some([1, 1]), &sounds),
        Ok(true)
    );
    assert_eq!(pings(), 2);
}

#[test]
fn a_dirty_portrait_with_mixed_starts_and_bmp_2002_checks_placement_on_found_tiles() {
    // Make tile (1, 1) its own unreachable room (room 9) full of a solid
    // object so starting at (1, 1) finds no tile while all other starts
    // land in room 1. With slot 1 dirty and BMP_ 2002, view_centre = None
    // succeeds whenever `portrait_place(&World)` passes on the found tiles
    // (statically proving placing and removing unput leave identical state)
    // and fails loudly if placement would have side effects.
    let setup = |indicators: [u16; 2]| {
        let (mut fx, widget_file, record) = visitors();
        fx.world.indicators = indicators;
        fx.world.dirty_portraits.insert(WIDGET);
        fx.program
            .libraries
            .get_mut(&widget_file)
            .unwrap()
            .has_portrait_bitmap = true;
        // Put room 9 on tile (1, 1) and occupy tile (1, 1) in the object grid.
        fx.world.rooms.map[0][64 + 1] = 9;
        let grid = &mut fx.world.arrays.get_mut(&3).unwrap().data;
        grid[(64 + 1) * 2..((64 + 1) * 2 + 2)].copy_from_slice(&(LAMP as i16).to_le_bytes());
        (fx, record)
    };
    let (mut fx_ok, record) = setup([99, 0]);
    assert_eq!(inform_from(&mut fx_ok, 2, record, None), Ok(true));
    assert_eq!(fx_ok.world.portraits, [WIDGET].into());
    assert!(fx_ok.world.dirty_portraits.is_empty());

    // Without indicators, the placed branch differs from the unput branch
    // (creating indicators vs not), so view_centre = None fails loudly.
    let (mut fx_no_ind, record) = setup([0, 0]);
    let fault = inform_from(&mut fx_no_ind, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x4e96dd"), "{fault}");

    // When a room's occupant mark would change on the found tiles, placement
    // differs from the unput branch and fails loudly rather than silently
    // taking the unput branch.
    let (mut fx_room, record) = setup([99, 0]);
    fx_room.world.neighbors.slots[record as usize - 1]
        .as_mut()
        .unwrap()
        .data[65] = 0;
    fx_room
        .world
        .rooms
        .rooms
        .insert(1, sims_sim::rooms::Room::default());
    let fault = inform_from(&mut fx_room, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(fault.contains("0x4e96dd"), "{fault}");

    // Without BMP_ 2002, mixed starts fail loudly on 0x4e96dd / 0x4e9706.
    let (mut fx_no_bmp, _widget_file, record) = visitors();
    fx_no_bmp.world.rooms.map[0][64 + 1] = 9;
    let grid = &mut fx_no_bmp.world.arrays.get_mut(&3).unwrap().data;
    grid[(64 + 1) * 2..((64 + 1) * 2 + 2)].copy_from_slice(&(LAMP as i16).to_le_bytes());
    let fault = inform_from(&mut fx_no_bmp, 2, record, None)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains("0x4e96dd") && fault.contains("0x4e9706"),
        "{fault}"
    );
}

#[test]
fn a_read_only_off_lot_portrait_never_creates_a_temporary_person() {
    // 0x4e8cc6 checks resource writability before calling 0x4e9652: a dirty
    // off-lot read-only resource with BMP_ 2002 skips 0x4e9652 and loads BMP_
    // 2002 at 0x4e9338 (zero builds, zero RNG draws, clearing dirty).
    let (mut fx, widget_file, record) = visitors();
    fx.world.size = 2; // would hit no-tile if it tried to build
    fx.world.dirty_portraits.insert(WIDGET);
    let lib = fx.program.libraries.get_mut(&widget_file).unwrap();
    lib.writable = false;
    lib.has_portrait_bitmap = true;
    lib.sounds.insert(5, "init_ping".into());
    fx.add(
        widget_file,
        4096,
        tree(vec![ins(23, RETURN_TRUE, ERROR, [5, 0, 0, 0, 0, 0, 0, 0])]),
    );
    let before = as_with_no_portrait(&fx, record);
    let sounds = std::cell::RefCell::new(Vec::new());
    assert_eq!(
        inform_hearing(&mut fx, 2, record, Some([1, 1]), &sounds),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.dirty_portraits.is_empty());
    assert!(fx.world.saved_portraits.is_empty());
    assert!(sounds.borrow().is_empty(), "{:?}", sounds.borrow());
    assert_eq!(rest(&fx), before);

    // Without BMP_ 2002, a read-only off-lot resource skips 0x4e9652 on both
    // 0x4e92f2 and 0x4e938c, leaving slot 1 empty and dirty unchanged.
    let (mut fx, widget_file, record) = visitors();
    fx.world.size = 2;
    fx.world.dirty_portraits.insert(WIDGET);
    fx.program.libraries.get_mut(&widget_file).unwrap().writable = false;
    let before = as_with_no_portrait(&fx, record);
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert!(fx.world.portraits.is_empty());
    assert_eq!(fx.world.dirty_portraits, [WIDGET].into());
    assert!(fx.world.saved_portraits.is_empty());
    assert_eq!(rest(&fx), before);
}

#[test]
fn a_successful_clean_build_saves_bmp_2002_for_a_later_no_tile_fallback() {
    // Start with a writable character file lacking BMP_ 2002.
    // 1. Clean-slot dialog succeeds -> 0x4e938c passes save = 1 -> 0x4e8d2e →
    //    0x4e9000 saves BMP_ 2002 (`saved_portraits`).
    let (mut fx, _, record) = visitors();
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert_eq!(fx.world.saved_portraits, [WIDGET].into());

    // 2. Invalidate portrait (as 0x4539cf → 0x4eb0eb does) and shrink the lot
    //    so no free tile exists. The next dialog runs one build at 0x4e92f2,
    //    removes unput at 0x4e9706, and falls back to the saved BMP_ 2002 at
    //    0x4e9338 without failing.
    fx.world.portraits.remove(&WIDGET);
    fx.world.dirty_portraits.insert(WIDGET);
    fx.world.dialogs.records.clear();
    fx.world.size = 2;
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.dirty_portraits.is_empty());
    assert_eq!(fx.world.saved_portraits, [WIDGET].into());

    // Contrast: a dirty-slot build (0x4e92f2) passes save = 0 and does NOT
    // write BMP_ 2002, so on a later blocked lot both 0x4e92f2 and 0x4e938c
    // find no tile, remove unput at 0x4e9706, and leave slot 1 empty and dirty.
    let (mut fx, _, record) = visitors();
    fx.world.dirty_portraits.insert(WIDGET);
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert!(fx.world.saved_portraits.is_empty());
    fx.world.portraits.remove(&WIDGET);
    fx.world.dirty_portraits.insert(WIDGET);
    fx.world.dialogs.records.clear();
    fx.world.size = 2;
    assert_eq!(inform_from(&mut fx, 2, record, Some([1, 1])), Ok(true));
    assert!(fx.world.portraits.is_empty());
    // Construction's `0x588930 -> 0x58d4e1 -> 0x4eb0cc -> 0x4e897e` clears the
    // dirty flag on an unbound head slot even when placement finds no tile.
    assert!(fx.world.dirty_portraits.is_empty());
    assert!(fx.world.saved_portraits.is_empty());
}

#[test]
fn a_saved_bmp_2002_applies_to_every_definition_in_the_same_file() {
    // 0x4e904e..0x4e90ab writes BMP_ 2002 into the type's resource file, and
    // 0x4e926a..0x4e9287 reads BMP_ 2002 back from that file. A clean build
    // for one person definition in a shared writable file therefore makes
    // BMP_ 2002 available to another person definition in that same file.
    const SIBLING: u32 = 0x5555_0002;
    let (mut fx, widget_file, widget_record) = visitors();
    let index = fx.catalog.objects.len();
    fx.catalog.objects.push(sims_content::Object {
        guid: SIBLING,
        resource: widget_file,
        chunk: 129,
        kind: 2,
        master: 0,
    });
    fx.catalog.by_guid.insert(SIBLING, index);
    let lib = fx.program.libraries.get_mut(&widget_file).unwrap();
    let words = *lib.definitions.get(&128).unwrap();
    lib.definitions.insert(129, words);
    let callbacks = lib.callbacks.get(&128).unwrap().clone();
    lib.callbacks.insert(129, callbacks);
    crate::person_factory::equip(&mut fx, SIBLING, "sibling", 12);
    let sibling_record = fx.world.neighbors.slots.len() as i16;

    // 1. Clean off-lot build for WIDGET writes BMP_ 2002 into `widget_file`.
    assert_eq!(
        inform_from(&mut fx, 2, widget_record, Some([1, 1])),
        Ok(true)
    );
    assert_eq!(fx.world.saved_portraits, [WIDGET].into());

    // 2. Clean off-lot dialog for SIBLING (in `widget_file`) now loads that
    //    BMP_ 2002 directly with zero builds even on a blocked lot.
    fx.world.dialogs.records.clear();
    fx.world.size = 2;
    let before = as_with_no_portrait(&fx, sibling_record);
    assert_eq!(
        inform_from(&mut fx, 2, sibling_record, Some([1, 1])),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&SIBLING));
    assert_eq!(rest(&fx), before);

    // 3. Dirty off-lot dialog for SIBLING on the blocked lot falls back to
    //    the shared file's saved BMP_ 2002 cleanly at 0x4e9338.
    fx.world.portraits.remove(&SIBLING);
    fx.world.dirty_portraits.insert(SIBLING);
    fx.world.dialogs.records.clear();
    assert_eq!(
        inform_from(&mut fx, 2, sibling_record, Some([1, 1])),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&SIBLING));
    assert!(!fx.world.dirty_portraits.contains(&SIBLING));
}

#[test]
fn a_temporary_persons_reset_invalidating_pd_68_leaves_slot_1_empty_and_dirty() {
    // 0x4e8d2e (save BMP_ 2002) and 0x4e8d6d (populate slot 1, clearing dirty)
    // run BEFORE 0x4e8fdb removes the temporary person. If Reset writes person
    // data word 68, 74, or 84, it invalidates the freshly populated slot.
    let setup = |has_bmp: bool, dirty: bool| {
        let (mut fx, widget_file, record) = visitors();
        // Register a Reset callback (OBJf entry 3, tree 4100) that writes
        // My Person Data 68 := 1. remove_now runs Reset at 0x4e8fdb AFTER
        // 0x4e8d2e (save BMP_ 2002) and 0x4e8d6d (populate slot 1).
        crate::visits::function(&mut fx, widget_file, 3, 4100);
        fx.add(
            widget_file,
            4100,
            tree(vec![assign(
                scope::MY_PERSON,
                68,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
        );
        fx.program
            .libraries
            .get_mut(&widget_file)
            .unwrap()
            .has_portrait_bitmap = has_bmp;
        if dirty {
            fx.world.dirty_portraits.insert(WIDGET);
        }
        (fx, record)
    };

    // 1. Clean slot without BMP_ 2002 (0x4e938c -> 0x4e8c5d(index, 1, 1)):
    //    BMP_ 2002 is saved before remove_now, then Reset invalidates slot 1,
    //    leaving slot 1 empty and dirty.
    let (mut fx_clean, record) = setup(false, false);
    assert_eq!(
        inform_from(&mut fx_clean, 2, record, Some([1, 1])),
        Ok(true)
    );
    assert_eq!(fx_clean.world.saved_portraits, [WIDGET].into());
    assert!(!fx_clean.world.portraits.contains(&WIDGET));
    assert!(fx_clean.world.dirty_portraits.contains(&WIDGET));

    // 2. Dirty slot WITH BMP_ 2002 (0x4e92f2 -> Reset invalidates slot 1 ->
    //    0x4e9315 sees empty slot 1 -> 0x4e9338 reloads BMP_ 2002, clearing
    //    dirty).
    let (mut fx_dirty_bmp, record) = setup(true, true);
    assert_eq!(
        inform_from(&mut fx_dirty_bmp, 2, record, Some([1, 1])),
        Ok(true)
    );
    assert!(fx_dirty_bmp.world.portraits.contains(&WIDGET));
    assert!(!fx_dirty_bmp.world.dirty_portraits.contains(&WIDGET));

    // 3. Dirty slot WITHOUT BMP_ 2002 (0x4e92f2 -> Reset invalidates slot 1 ->
    //    0x4e9315 sees empty slot 1 -> 0x4e938c runs a second build with
    //    save = 1, saving BMP_ 2002 before Reset invalidates slot 1 again).
    let (mut fx_dirty_nobmp, record) = setup(false, true);
    assert_eq!(
        inform_from(&mut fx_dirty_nobmp, 2, record, Some([1, 1])),
        Ok(true)
    );
    assert_eq!(fx_dirty_nobmp.world.saved_portraits, [WIDGET].into());
    assert!(!fx_dirty_nobmp.world.portraits.contains(&WIDGET));
    assert!(fx_dirty_nobmp.world.dirty_portraits.contains(&WIDGET));
}

#[test]
fn a_dirty_build_whose_reset_saves_a_siblings_bmp_2002_loads_it_at_0x4e9338() {
    // 0x4e9338 looks up BMP_ 2002 in the resource file AFTER 0x4e92f2 returns:
    // if WIDGET's Reset invalidates slot 1 AND triggers a clean dialog for a
    // sibling person definition in the same writable IFF (saving BMP_ 2002 into
    // that file), 0x4e9338 loads that newly saved bitmap cleanly.
    const SIBLING: u32 = 0x5555_0003;
    let (mut fx, widget_file, widget_record) = visitors();
    let index = fx.catalog.objects.len();
    fx.catalog.objects.push(sims_content::Object {
        guid: SIBLING,
        resource: widget_file,
        chunk: 129,
        kind: 2,
        master: 0,
    });
    fx.catalog.by_guid.insert(SIBLING, index);
    let lib = fx.program.libraries.get_mut(&widget_file).unwrap();
    let words = *lib.definitions.get(&128).unwrap();
    lib.definitions.insert(129, words);
    let callbacks = lib.callbacks.get(&128).unwrap().clone();
    lib.callbacks.insert(129, callbacks);
    crate::person_factory::equip(&mut fx, SIBLING, "sibling", 12);
    let sibling_record = fx.world.neighbors.slots.len() as i16;

    // Give WIDGET a Reset callback (OBJf entry 3, tree 4100) that writes
    // pd[68] := 1 (invalidating WIDGET's freshly built slot 1) and runs an
    // informing dialog for SIBLING (saving BMP_ 2002 into `widget_file`).
    crate::visits::function(&mut fx, widget_file, 3, 4100);
    fx.add(
        widget_file,
        4100,
        tree(vec![
            assign(scope::MY_PERSON, 68, scope::LITERAL, 1, 1),
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, sibling_record, 2),
            ins(
                DIALOG,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
            ),
        ]),
    );
    fx.world.dirty_portraits.insert(WIDGET);
    assert_eq!(
        inform_from(&mut fx, 2, widget_record, Some([1, 1])),
        Ok(true)
    );
    assert!(fx.world.saved_portraits.contains(&SIBLING));
    assert!(fx.world.portraits.contains(&WIDGET));
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));
}

#[test]
fn a_dirty_no_tile_build_succeeds_when_reset_populates_slot_1() {
    // 0x4e9706 removes the temporary person unput, running Reset while the
    // person is still registered. If Reset requests a portrait of that same
    // person (populating slot 1 from the on-lot temporary person), 0x4e9315
    // sees slot 1 non-null and succeeds with no BMP_ 2002 on disk.
    let (mut fx, widget_file, widget_record) = visitors();
    fx.world.size = 2; // no free tile -> NotFound -> 0x4e9706
    fx.world.dirty_portraits.insert(WIDGET);
    // Reset callback (OBJf entry 3, tree 4100): show a mode-2 informing dialog
    // naming WIDGET's own neighbor record while the temporary person is still
    // registered in `world.people`, populating slot 1 via the on-lot path.
    crate::visits::function(&mut fx, widget_file, 3, 4100);
    fx.add(
        widget_file,
        4100,
        tree(vec![
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, widget_record, 1),
            ins(
                DIALOG,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
            ),
        ]),
    );
    assert_eq!(
        inform_from(&mut fx, 2, widget_record, Some([1, 1])),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&WIDGET));
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));
    // Construction's `0x588930 -> 0x58d4e1 -> 0x4eb0cc -> 0x4e897e` clears the
    // dirty flag before Reset runs, so Reset's nested dialog takes the clean
    // `0x4e938c` (`save = 1`) path and saves `BMP_` 2002.
    assert_eq!(fx.world.saved_portraits, [WIDGET].into());
}
