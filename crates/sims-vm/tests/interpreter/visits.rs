//! Load step 13 and what it needs: the visit of an object, taking an
//! object off its tile and putting it back, the frame storage, and the
//! primitives the visits reach.

use crate::factory::{WIDGET, widgets};
use crate::rooms::build;
use crate::support::*;
use sims_content::People;
use sims_formats::execution::ObjectFunction;
use sims_formats::saved_objects::Multipart;
use sims_vm::{Asked, INDICATORS, Interpreter, Status, Trap};
use std::cell::RefCell;

const BUDGET: usize = 1000;
/// Trees of the widget's file.
const LOAD: u16 = 4200;
const PLACED: u16 = 4201;
const LEAVING: u16 = 4202;
const RESET: u16 = 4203;

/// Makes `tree` the function at `place` of the one type in `file`.
pub fn function(fx: &mut Fixture, file: usize, place: usize, tree: u16) {
    let callbacks = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    if callbacks.len() <= place {
        callbacks.resize(
            place + 1,
            ObjectFunction {
                condition: 0,
                action: 0,
            },
        );
    }
    callbacks[place].action = tree;
}

/// A lot with rooms, the widget type, and the two indicator types the
/// lot creates when something first comes onto it. The lamp stays as a
/// notebook, outside the registration list: the widget's Load tree sets
/// the lamp's attribute 0 to 2, its function 10 tree copies its room to
/// the lamp's attribute 1, and its function 9 tree copies its room to the
/// lamp's attribute 2.
pub fn lot() -> (Fixture, usize) {
    let (mut fx, file) = widgets();
    build(&mut fx);
    for grid in [3, 103] {
        fx.world.arrays.insert(grid, crate::rooms::array(2, 0));
    }
    for guid in INDICATORS {
        let indicator = fx.add_type(guid, 4, 0, 4097);
        fx.add(
            indicator,
            4097,
            tree(vec![ins(256, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
        );
    }
    fx.add(file, LOAD, note(0, scope::LITERAL, 2));
    fx.add(file, LEAVING, note(1, scope::MY_DATA, 29));
    fx.add(file, PLACED, note(2, scope::MY_DATA, 29));
    function(&mut fx, file, 2, LOAD);
    function(&mut fx, file, 9, PLACED);
    function(&mut fx, file, 10, LEAVING);
    // The visits here are the widgets'.
    fx.world.order.clear();
    fx.world.objects.remove(&SIM);
    fx.lamp_mut().attributes = vec![0; 8];
    (fx, file)
}

/// A tree that writes a value into one of the lamp's attributes.
fn note(attribute: i16, from_scope: u8, from: i16) -> sims_formats::behavior::Behavior {
    tree(vec![
        assign(scope::STACK_OBJECT, 0, scope::LITERAL, LAMP as i16, 1),
        assign(
            scope::STACK_ATTRIBUTES,
            attribute,
            from_scope,
            from,
            RETURN_TRUE,
        ),
    ])
}

/// What the widget's trees wrote down: Load, leaving, placed, Reset.
fn notes(fx: &Fixture) -> [i16; 4] {
    let noted = &fx.lamp().attributes;
    [noted[0], noted[1], noted[2], noted[3]]
}

/// The ID of the person some tests add.
const PERSON: u16 = 50;

/// Creates a widget and stands it on a tile of level 1, in front of
/// whatever stands there.
pub fn stand(fx: &mut Fixture, tile: [i32; 2]) -> u16 {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let id = interpreter.create(&mut fx.world, WIDGET).unwrap();
    let cell = ((tile[0] * 64 + tile[1]) * 2) as usize;
    let grid = &mut fx.world.arrays.get_mut(&3).unwrap().data;
    let behind = u16::from_le_bytes([grid[cell], grid[cell + 1]]);
    grid[cell..cell + 2].copy_from_slice(&id.to_le_bytes());
    let position = [tile[0] * 16 + 8, tile[1] * 16 + 8, 1];
    let room = fx.world.room_at(position).unwrap();
    let widget = fx.world.objects.get_mut(&id).unwrap();
    widget.position = position;
    widget.data[29] = room;
    widget.slots[0].object = behind as i16;
    if behind != 0 {
        let other = fx.world.objects.get_mut(&behind).unwrap();
        (other.data[14], other.data[26]) = (0, id as i16);
    }
    id
}

fn grid(fx: &Fixture, tile: [usize; 2]) -> u16 {
    let cell = (tile[0] * 64 + tile[1]) * 2;
    let grid = &fx.world.arrays[&3].data;
    u16::from_le_bytes([grid[cell], grid[cell + 1]])
}

fn visit(fx: &mut Fixture, id: u16) -> Vec<(u16, usize)> {
    let asked = RefCell::new(Vec::new());
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.asked = Some(&asked);
    interpreter
        .visit(&mut fx.world, id, &People::default())
        .unwrap();
    asked
        .into_inner()
        .into_iter()
        .map(|Asked { object, function }| (object, function))
        .collect()
}

#[test]
fn an_object_whose_type_did_not_change_runs_only_its_load_function() {
    let (mut fx, _) = lot();
    let id = stand(&mut fx, [2, 2]);
    let before = fx.world.objects[&id].clone();
    fx.world.objects.get_mut(&id).unwrap().data[60] = 1;
    assert_eq!(visit(&mut fx, id), [(id, 2)]);
    let widget = &fx.world.objects[&id];
    assert_eq!(notes(&fx), [2, 0, 0, 0]);
    // Its thread, attributes and place are as they were.
    assert_eq!(widget.stack.len(), before.stack.len());
    assert_eq!(widget.attributes, before.attributes);
    assert_eq!(widget.position, before.position);
    assert_eq!(widget.countdown(), before.countdown());
    assert_eq!(grid(&fx, [2, 2]), id);
    // The footprint is worked out again: 16 wide around the position.
    assert_eq!(widget.footprint, [32, 32, 48, 48]);
    // Data word 60 shows in the scheduler flags.
    assert_eq!(widget.flags() & 2, 2);
    assert_eq!(fx.world.indicators, [0, 0]);
}

#[test]
fn an_object_on_level_0_goes_to_level_1() {
    let (mut fx, _) = lot();
    let id = stand(&mut fx, [2, 2]);
    fx.world.objects.get_mut(&id).unwrap().position[2] = 0;
    visit(&mut fx, id);
    assert_eq!(fx.world.objects[&id].position, [40, 40, 1]);
}

#[test]
fn a_changed_object_with_no_reset_tree_is_rebuilt_where_it_stands() {
    let (mut fx, _) = lot();
    let id = stand(&mut fx, [2, 2]);
    fx.world.changed_types.insert(WIDGET);
    let widget = fx.world.objects.get_mut(&id).unwrap();
    widget.attributes[5] = 99;
    widget.data[8] = 0x100;
    let room = widget.data[29];
    assert_eq!(room, 1);
    // Load, leaving, Init inside the rebuild, placed, and Init again.
    // Coming back onto the lot from outside makes the lot's two
    // indicators, whose Init is asked too.
    assert_eq!(
        visit(&mut fx, id),
        [
            (id, 2),
            (id, 10),
            (id, 0),
            (id, 9),
            (id + 1, 0),
            (id + 2, 0),
            (id, 0)
        ]
    );
    // It left from its room and was asked function 9 back in it.
    assert_eq!(notes(&fx), [2, room, room, 0]);
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.position, [40, 40, 1]);
    assert_eq!(grid(&fx, [2, 2]), id);
    assert_eq!(
        (
            widget.data[2],
            widget.data[3],
            widget.data[14],
            widget.data[26]
        ),
        (0, -1, -1, 0)
    );
    assert_eq!(widget.data[29], room);
    // Its storage was reset: the attributes are Init's, flag 0x100 is
    // gone, and the thread sleeps in a new Main.
    assert_eq!(widget.attributes[5], 0);
    assert_eq!(widget.attributes[0], 7);
    assert_eq!(widget.data[8] & 0x100, 0);
    assert_eq!(widget.stack.len(), 2);
    // The reset ends by dropping the scheduler's countdown.
    assert_eq!(widget.countdown(), Some(-1));
    // The indicators go to the head of the registration list.
    assert_eq!(fx.world.indicators, [id + 1, id + 2]);
    assert_eq!(fx.world.order.len(), 3);
    assert_eq!(fx.world.order[2], id);
}

#[test]
fn an_object_that_is_off_the_lot_is_rebuilt_there() {
    let (mut fx, _) = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let id = interpreter.create(&mut fx.world, WIDGET).unwrap();
    fx.world.changed_types.insert(WIDGET);
    // Function 10 is asked, function 9 is not, and no indicator is made.
    assert_eq!(visit(&mut fx, id), [(id, 2), (id, 10), (id, 0), (id, 0)]);
    assert_eq!(fx.world.objects[&id].position, [-16, -16, 1]);
    assert_eq!(fx.world.objects[&id].data[29], -5);
    assert_eq!(fx.world.indicators, [0, 0]);
}

#[test]
fn a_rebuilt_object_goes_back_in_front_of_the_others_on_its_tile() {
    let (mut fx, _) = lot();
    let behind = stand(&mut fx, [2, 3]);
    let front = stand(&mut fx, [2, 3]);
    fx.world.changed_types.insert(WIDGET);
    // The one behind is rebuilt: it leaves the tile, and comes back in
    // front.
    visit(&mut fx, behind);
    assert_eq!(grid(&fx, [2, 3]), behind);
    let (a, b) = (&fx.world.objects[&behind], &fx.world.objects[&front]);
    assert_eq!(a.slots[0].object, front as i16);
    assert_eq!((a.data[14], a.data[26]), (-1, 0));
    assert_eq!(b.slots[0].object, 0);
    assert_eq!((b.data[14], b.data[26]), (0, behind as i16));
    assert_eq!(a.position, b.position);
}

#[test]
fn a_changed_object_with_a_reset_tree_runs_it_and_stays() {
    let (mut fx, file) = lot();
    fx.add(file, RESET, note(3, scope::LITERAL, 6));
    function(&mut fx, file, 3, RESET);
    let id = stand(&mut fx, [2, 2]);
    fx.world.changed_types.insert(WIDGET);
    fx.world.objects.get_mut(&id).unwrap().attributes[5] = 99;
    assert_eq!(visit(&mut fx, id), [(id, 2), (id, 3), (id, 0)]);
    // Not taken off the lot, storage kept, Main restarted.
    assert_eq!(notes(&fx), [2, 0, 0, 6]);
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.attributes[5], 99);
    assert_eq!(widget.stack.len(), 1);
    assert_eq!(widget.countdown(), Some(-1));
    assert_eq!(grid(&fx, [2, 2]), id);
}

#[test]
fn a_multi_tile_object_is_rebuilt_as_a_group() {
    let (mut fx, file) = lot();
    let lead = stand(&mut fx, [2, 2]);
    let tile = stand(&mut fx, [3, 2]);
    // The definition is in a group (word 10).
    fx.words(file)[10] = 5;
    for (id, parent, place) in [(lead, 0, 0), (tile, lead as i16, 1)] {
        fx.world.objects.get_mut(&id).unwrap().multipart = Some(Multipart {
            offset: [place, 0, 0],
            group: [place, 0, 0],
            parent,
        });
    }
    fx.world.next_parts.insert(lead, tile);
    fx.world.changed_types.insert(WIDGET);
    // Visiting one tile rebuilds both: every tile is asked function 10,
    // then each runs Init, then every tile is asked function 9. The
    // first tile to stand on the lot again makes the indicators.
    assert_eq!(
        visit(&mut fx, tile),
        [
            (tile, 2),
            (lead, 10),
            (tile, 10),
            (lead, 0),
            (tile, 0),
            (tile + 1, 0),
            (tile + 2, 0),
            (lead, 9),
            (tile, 9),
            (tile, 0)
        ]
    );
    assert_eq!(fx.world.objects[&lead].position, [40, 40, 1]);
    assert_eq!(fx.world.objects[&tile].position, [56, 40, 1]);
    assert_eq!((grid(&fx, [2, 2]), grid(&fx, [3, 2])), (lead, tile));
    // A tile of a group does not run Init when its own storage resets,
    // only as part of the group, so both ran it once here, and the
    // visited one once more.
    assert_eq!(fx.world.objects[&lead].attributes[0], 7);
}

#[test]
fn a_person_referring_to_a_changed_object_stops_its_visit() {
    let (mut fx, _) = lot();
    let id = stand(&mut fx, [2, 2]);
    fx.world.changed_types.insert(WIDGET);
    let mut sim = object(PERSON, SIM_GUID);
    let mut person = person();
    person.active.target = id as i16;
    sim.person = Some(person);
    fx.world.objects.insert(PERSON, sim);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter
        .visit(&mut fx.world, id, &People::default())
        .unwrap_err();
    assert!(
        fault.to_string().contains("person 50 refers to object"),
        "{fault}"
    );
}

/// The GUID of the person type some tests add, and its trees.
const RESIDENT: u32 = 0x7777;
const RESIDENT_INIT: u16 = 4300;
const RESIDENT_MAIN: u16 = 4301;
const RESIDENT_RESET: u16 = 4302;
const RESIDENT_LOAD: u16 = 4303;
const CANCELLED: u16 = 4204;

/// Adds a person type with Init, Main, Load and Reset trees, a person of
/// it standing on tile (3, 3), and what a visit needs of the neighborhood:
/// record 9 of revision 10, whose person data word n holds 1000 + n, and
/// the type's profile. Init writes 5 to the lamp's attribute 4, Reset
/// writes 6 to its attribute 5, and Load would write 7 to its attribute 6.
fn resident(fx: &mut Fixture) -> People {
    let file = fx.add_type(RESIDENT, 2, RESIDENT_INIT, RESIDENT_MAIN);
    fx.add(file, RESIDENT_INIT, note(4, scope::LITERAL, 5));
    fx.add(
        file,
        RESIDENT_MAIN,
        tree(vec![ins(256, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
    );
    fx.add(file, RESIDENT_RESET, note(5, scope::LITERAL, 6));
    fx.add(file, RESIDENT_LOAD, note(6, scope::LITERAL, 7));
    function(fx, file, 2, RESIDENT_LOAD);
    function(fx, file, 3, RESIDENT_RESET);
    fx.world.types.insert(30, RESIDENT);
    let mut sim = object(PERSON, RESIDENT);
    let mut data = person();
    data.data[73] = 0;
    sim.person = Some(data);
    sim.position = [56, 56, 1];
    sim.data[1] = 0;
    (sim.data[2], sim.data[3], sim.data[14], sim.data[26]) = (0, -1, -1, 0);
    sim.slots.iter_mut().for_each(|slot| slot.object = 0);
    sim.registry_word = Some(0xffff);
    fx.world.objects.insert(PERSON, sim);
    fx.world.order.push(PERSON);
    fx.thread(PERSON, RESIDENT_MAIN, &[0; 4]);
    let cell = (3 * 64 + 3) * 2;
    let grid = &mut fx.world.arrays.get_mut(&3).unwrap().data;
    grid[cell..cell + 2].copy_from_slice(&PERSON.to_le_bytes());
    People {
        records: vec![sims_content::Record {
            id: 9,
            guid: RESIDENT,
            revision: 10,
            data: (0..256).map(|n| 1000 + n).collect(),
        }],
        profiles: [(
            RESIDENT,
            sims_content::Profile {
                gender: Some("Female".into()),
                age: Some(" 27 years".into()),
                skin: Some("DRK".into()),
                tones: ["unknown", "lgt", "med", "drk"].map(String::from).to_vec(),
            },
        )]
        .into(),
        ..People::default()
    }
}

fn visit_with(fx: &mut Fixture, id: u16, people: &People) -> Vec<(u16, usize)> {
    let asked = RefCell::new(Vec::new());
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.asked = Some(&asked);
    interpreter.visit(&mut fx.world, id, people).unwrap();
    asked
        .into_inner()
        .into_iter()
        .map(|Asked { object, function }| (object, function))
        .collect()
}

#[test]
fn a_person_takes_its_lasting_fields_from_its_neighborhood_record() {
    let (mut fx, _) = lot();
    let people = resident(&mut fx);
    let before = fx.world.objects[&PERSON].clone();
    // Its type did not change: no function is asked, not even Load.
    assert_eq!(visit_with(&mut fx, PERSON, &people), []);
    let after = &fx.world.objects[&PERSON];
    let (old, new) = (
        &before.person.as_ref().unwrap().data,
        &after.person.as_ref().unwrap().data,
    );
    let kept = sims_vm::persistent_fields(10);
    assert_eq!(kept.len(), 48);
    for (word, (&new, &old)) in new.iter().zip(old).enumerate() {
        let expected = match word {
            31 => 9,
            word if kept.contains(&word) => 1000 + word as i16,
            _ => old,
        };
        assert_eq!(new, expected, "person data word {word}");
    }
    assert_eq!(after.stack.len(), before.stack.len());
    assert_eq!(after.stack[0].node, before.stack[0].node);
    assert_eq!(after.data[29], 1);
    // A person is 6 wide.
    assert_eq!(after.footprint, [53, 53, 59, 59]);
}

#[test]
fn a_record_keeps_more_fields_from_each_revision_on() {
    assert_eq!(sims_vm::persistent_fields(0), []);
    assert_eq!(sims_vm::persistent_fields(1).len(), 29);
    assert_eq!(sims_vm::persistent_fields(1)[..3], [2, 3, 4]);
    assert_eq!(sims_vm::persistent_fields(1)[26..], [58, 60, 61]);
    assert_eq!(sims_vm::persistent_fields(2)[29..], [56, 57, 63]);
    assert_eq!(sims_vm::persistent_fields(7)[37..], [54, 55, 75, 76]);
    assert_eq!(
        sims_vm::persistent_fields(99),
        sims_vm::persistent_fields(10)
    );
}

#[test]
fn a_changed_person_resets_in_its_own_way_and_then_runs_init() {
    let (mut fx, file) = lot();
    let people = resident(&mut fx);
    let widget = stand(&mut fx, [2, 2]);
    // The widget notes who cancelled an action queued on it.
    fx.add(file, CANCELLED, note(7, scope::STACK_OBJECT, 0));
    function(&mut fx, file, 4, CANCELLED);
    fx.world.changed_types.insert(RESIDENT);
    let sim = fx.world.objects.get_mut(&PERSON).unwrap();
    (sim.data[9], sim.data[34]) = (3, 4);
    let person = sim.person.as_mut().unwrap();
    let mut queued = person.last.clone();
    queued.target = widget as i16;
    let mut idle = person.last.clone();
    idle.target = 0;
    person.queue = vec![idle, queued];
    person
        .motive_deltas
        .push(sims_formats::saved_objects::MotiveDelta {
            motive: 5,
            delta: 1.0,
            stop_at: 50.0,
        });
    let motives = person.motives;
    // Reset, the cancelled action's target, then Init. Never Load.
    assert_eq!(
        visit_with(&mut fx, PERSON, &people),
        [(PERSON, 3), (widget, 4), (PERSON, 0)]
    );
    let noted = &fx.lamp().attributes;
    // The note tree leaves the lamp as its stack object, so function 4
    // wrote the lamp's own ID; what matters is that it ran.
    assert_eq!((noted[4], noted[5], noted[6]), (5, 6, 0));
    assert_ne!(noted[7], 0);
    let sim = &fx.world.objects[&PERSON];
    let person = sim.person.as_ref().unwrap();
    for word in [0, 22, 23, 30, 33, 40, 59, 71, 72, 73] {
        assert_eq!(person.data[word], 0, "person data word {word}");
    }
    // The profile: female, 27, the third tone.
    assert_eq!(
        (person.data[65], person.data[58], person.data[60]),
        (1, 27, 3)
    );
    assert_eq!((sim.data[9], sim.data[34]), (0, 0));
    assert!(person.queue.is_empty() && person.motive_deltas.is_empty());
    assert_eq!(person.motives, motives);
    // The thread restarted at Main, with no stack object.
    assert_eq!(sim.stack.len(), 1);
    let frame = &sim.stack[0];
    assert_eq!(
        (frame.tree, frame.node, frame.stack_object),
        (RESIDENT_MAIN, 0, 0)
    );
    assert_eq!(sim.position, [56, 56, 1]);
    assert_eq!(grid(&fx, [3, 3]), PERSON);
}

#[test]
fn a_profile_string_that_is_missing_leaves_its_word() {
    let profile =
        |gender: Option<&str>, age: Option<&str>, skin: Option<&str>| sims_content::Profile {
            gender: gender.map(String::from),
            age: age.map(String::from),
            skin: skin.map(String::from),
            tones: ["lgt", "lgt", "med"].map(String::from).to_vec(),
        };
    let reload = |profile: &sims_content::Profile| {
        let mut data: Vec<i16> = (0..256).collect();
        sims_vm::reload_profile(&mut data, profile).unwrap();
        (data[65], data[58], data[60], data[22], data[23])
    };
    assert_eq!(reload(&profile(None, None, None)), (65, 58, 60, 0, 0));
    // An unknown gender keeps the word; an unknown tone gives 0; an
    // empty age is 0.
    assert_eq!(
        reload(&profile(Some("robot"), Some(""), Some("blue"))),
        (65, 0, 0, 0, 0)
    );
    // The first tone name is never matched, so "lgt" is tone 1.
    assert_eq!(
        reload(&profile(Some("CATMALE"), Some("-3x"), Some("lgt"))),
        (4, -3, 1, 0, 0)
    );
    assert_eq!(reload(&profile(None, Some("+70000"), None)).1, 4464);
}

#[test]
fn a_persons_motive_curves_come_from_its_definition_or_its_age() {
    assert_eq!(sims_vm::curve_table(27, 0), (true, 501));
    assert_eq!(sims_vm::curve_table(18, 0), (true, 501));
    assert_eq!(sims_vm::curve_table(0, 0), (true, 501));
    assert_eq!(sims_vm::curve_table(9, 0), (true, 503));
    assert_eq!(sims_vm::curve_table(-1, 0), (true, 503));
    assert_eq!(sims_vm::curve_table(9, 4100), (false, 4100));
}

#[test]
fn a_person_the_reset_would_do_more_for_stops_its_visit() {
    let (mut fx, _) = lot();
    let people = resident(&mut fx);
    fx.world.changed_types.insert(RESIDENT);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fails = |change: &dyn Fn(&mut sims_sim::Object), people: &People| {
        let mut world = fx.world.clone();
        change(world.objects.get_mut(&PERSON).unwrap());
        interpreter
            .visit(&mut world, PERSON, people)
            .unwrap_err()
            .to_string()
    };
    let person = |change: &dyn Fn(&mut sims_formats::saved_objects::Person)| {
        fails(&|object| change(object.person.as_mut().unwrap()), &people)
    };
    let using = person(&|p| {
        p.object_uses.push(sims_formats::saved_objects::ObjectUse {
            target: 1,
            stack_depth: 1,
            unknown: 0,
        })
    });
    assert!(using.contains("that is using an object"), "{using}");
    let acting = person(&|p| p.active.target = 77);
    assert!(
        acting.contains("whose running action has a target"),
        "{acting}"
    );
    let routing = person(&|p| p.routing_frames = 1);
    assert!(routing.contains("in the middle of a route"), "{routing}");
    let wide = person(&|p| p.data[73] = 1);
    assert!(wide.contains("whose data word 73 is set"), "{wide}");
    let away = fails(&|object| object.position = [-16, -16, 1], &people);
    assert!(away.contains("inside an object or off the lot"), "{away}");
    // Without a neighborhood record, or with one that holds no person
    // data, the visit fails before anything else.
    let nobody = fails(&|_| {}, &People::default());
    assert!(nobody.contains("has no neighborhood record"), "{nobody}");
    let mut bare = people.clone();
    bare.records[0].revision = 0;
    let bare = fails(&|_| {}, &bare);
    assert!(bare.contains("holds no person data"), "{bare}");
    // The lot was not touched by any of it.
    assert!(fx.world.objects[&PERSON].stack.len() == 1);
}

#[test]
fn the_step_visits_people_in_their_turn_and_stops_at_a_visit_it_cannot_make() {
    let (mut fx, _) = lot();
    let first = stand(&mut fx, [2, 2]);
    let people = resident(&mut fx);
    let last = stand(&mut fx, [2, 3]);
    fx.world.order.retain(|&id| id != last);
    fx.world.order.push(last);
    fx.world.changed_types.insert(WIDGET);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    // With the person's record, all three are visited.
    let mut whole = fx.world.clone();
    let done = interpreter.visit_objects(&mut whole, &people);
    assert_eq!(done.stopped, None);
    assert_eq!(done.objects, [first, PERSON, last]);
    assert_eq!(done.reset, [first, last]);
    // Without it the step stops at the person, with the lot as the first
    // visit left it. The indicators that visit made are not visited:
    // they went to the head of the list.
    let mut expected = fx.world.clone();
    interpreter
        .visit(&mut expected, first, &People::default())
        .unwrap();
    let done = interpreter.visit_objects(&mut fx.world, &People::default());
    assert_eq!((done.objects, done.reset), (vec![first], vec![first]));
    let fault = done.stopped.unwrap();
    assert_eq!(fault.object, PERSON);
    assert!(
        fault.to_string().contains("has no neighborhood record"),
        "{fault}"
    );
    assert_eq!(fx.world.encode(), expected.encode());
    assert_eq!(fx.world.order.len(), 5);
}

#[test]
fn the_indicators_are_made_once() {
    let (mut fx, _) = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let made = interpreter.ensure_indicators(&mut fx.world).unwrap();
    assert_eq!(made, Some(fx.world.indicators));
    assert_ne!(fx.world.indicators[0], 0);
    let before = fx.world.encode();
    assert_eq!(interpreter.ensure_indicators(&mut fx.world), Ok(None));
    assert_eq!(fx.world.encode(), before);
}

#[test]
fn a_new_frames_locals_hold_what_an_earlier_frame_left() {
    let mut fx = Fixture::new();
    // The first tree leaves 55 in its local 1. The second, run the same
    // way, finds it there.
    fx.add(
        LAMP_FILE,
        4300,
        tree(vec![assign(
            scope::LOCALS,
            1,
            scope::LITERAL,
            55,
            RETURN_TRUE,
        )]),
    );
    fx.add(
        LAMP_FILE,
        4301,
        tree(vec![assign(
            scope::MY_ATTRIBUTES,
            0,
            scope::LOCALS,
            1,
            RETURN_TRUE,
        )]),
    );
    fx.lamp_mut().attributes = vec![0; 4];
    assert_eq!(fx.run_with(LAMP, 4301, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.lamp().attributes[0], 0);
    assert_eq!(fx.run_with(LAMP, 4300, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.run_with(LAMP, 4301, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.lamp().attributes[0], 55);
    // The storage holds the mark of a synchronous run, then the frame.
    let storage = &fx.world.frame_storage[&LAMP];
    assert_eq!(
        storage.words[..9],
        [-1, 0, 0, 0, 1, 0, 0, 0, 4301].map(Some)
    );
    assert_eq!(storage.words[8 + 8 + 4 + 1], Some(55));
    assert_eq!(storage.offsets, []);
}

#[test]
fn a_local_that_starts_on_a_pointer_cannot_be_read_until_it_is_written() {
    let mut fx = Fixture::new();
    // A tree with no words of its own calls another, whose header then
    // lies where the locals of a later, wider tree start.
    let narrow = |instructions| shaped(0x8002, 0, 0, 0, instructions);
    fx.add(
        LAMP_FILE,
        4300,
        narrow(vec![ins(4301, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
    );
    fx.add(
        LAMP_FILE,
        4301,
        narrow(vec![ins(2, RETURN_TRUE, RETURN_TRUE, expr(7, 0, 2, 7, 0))]),
    );
    let read = |local| {
        shaped(
            0x8002,
            0,
            0,
            8,
            vec![assign(
                scope::MY_ATTRIBUTES,
                0,
                scope::LOCALS,
                local,
                RETURN_TRUE,
            )],
        )
    };
    fx.lamp_mut().attributes = vec![0; 4];
    assert_eq!(fx.run_with(LAMP, 4300, 0, &[], BUDGET), Ok(true));
    // Local 0 is the inner frame's tree ID; locals 6 and 7 are the two
    // halves of its code pointer.
    fx.add(LAMP_FILE, 4302, read(0));
    assert_eq!(fx.run_with(LAMP, 4302, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.lamp().attributes[0], 4301);
    fx.add(LAMP_FILE, 4302, read(6));
    let text = trap(fx.run_with(LAMP, 4302, 0, &[], BUDGET)).to_string();
    assert!(text.contains("half of a pointer"), "{text}");
    // Assigning to it first makes it an ordinary local.
    fx.lamp_mut().stack.clear();
    fx.world.frame_storage.remove(&LAMP);
    assert_eq!(fx.run_with(LAMP, 4300, 0, &[], BUDGET), Ok(true));
    let mut both = read(6);
    both.instructions
        .insert(0, assign(scope::LOCALS, 6, scope::LITERAL, 12, 1));
    fx.add(LAMP_FILE, 4302, both);
    assert_eq!(fx.run_with(LAMP, 4302, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.lamp().attributes[0], 12);
}

#[test]
fn notify_ends_the_stack_objects_sleep() {
    let (mut fx, _) = widgets();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let id = interpreter.create(&mut fx.world, WIDGET).unwrap();
    assert_eq!(fx.world.objects[&id].countdown(), Some(19));
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(49, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, id as i16, &[], BUDGET), Ok(true));
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.countdown(), Some(0));
    assert_eq!(widget.stack[1].parameters, [0]);
    // Its next turn finishes the Sleep and starts the next one.
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(
        interpreter.turn(&mut fx.world, id, BUDGET),
        Ok(Status::Yielded)
    );
    assert_eq!(fx.world.objects[&id].countdown(), Some(19));
    // A missing stack object is the original's error 0x15.
    assert_eq!(code(fx.run_with(LAMP, 4096, 77, &[], BUDGET)), Some(0x15));
}

#[test]
fn an_object_notifies_itself_out_of_the_sleep_below_its_callback() {
    let (mut fx, file) = widgets();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let id = interpreter.create(&mut fx.world, WIDGET).unwrap();
    fx.add(
        file,
        4300,
        tree(vec![
            assign(scope::STACK_OBJECT, 0, scope::MY_DATA, 11, 1),
            ins(49, RETURN_TRUE, RETURN_FALSE, [0; 8]),
        ]),
    );
    assert_eq!(fx.run_with(id, 4300, 0, &[], BUDGET), Ok(true));
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.countdown(), Some(0));
    assert_eq!(widget.stack[1].parameters, [0]);
}

fn relationship(fx: &mut Fixture, opcode: u16, operands: [u8; 8]) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(opcode, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(LAMP, 4096, 3, &[3, 0, 0, 0], BUDGET)
}

#[test]
fn relationship_reads_and_writes_rows_kept_on_the_object() {
    let (mut fx, _) = widgets();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(3));
    fx.lamp_mut().temps = vec![0; 8];
    // Reading a number that is not there, with flag 1: the false branch,
    // and no row is made.
    assert_eq!(
        relationship(&mut fx, 26, [1, 0, 1, 0, 8, 0, 0, 0]),
        Ok(false)
    );
    assert!(fx.lamp().relationships.is_empty());
    // Writing column 1 from me to the stack object makes the row.
    assert_eq!(
        relationship(&mut fx, 26, [1, 0, 4, 0, 7, 0, 5, 0]),
        Ok(true)
    );
    let row = &fx.lamp().relationships[0];
    assert_eq!((row.target, row.values.as_slice()), (3, &[0, 5][..]));
    // Reading it back into temp 2.
    assert_eq!(
        relationship(&mut fx, 26, [1, 0, 0, 0, 8, 0, 2, 0]),
        Ok(true)
    );
    assert_eq!(fx.lamp().temps[2], 5);
    // The other direction is a row on the stack object.
    assert_eq!(
        relationship(&mut fx, 26, [0, 1, 4, 0, 7, 0, 9, 0]),
        Ok(true)
    );
    let row = &fx.world.objects[&3].relationships[0];
    assert_eq!((row.target, row.values.as_slice()), (1, &[9][..]));
    // The old form: write (byte 0) column 0 (byte 1) from parameter 1
    // (byte 2), from the stack object to the object parameter 0 names.
    fx.world.objects.get_mut(&3).unwrap().relationships.clear();
    assert_eq!(
        relationship(&mut fx, 24, [1, 0, 1, 2, 0, 0, 0, 0]),
        Ok(true)
    );
    let row = &fx.world.objects[&3].relationships[0];
    assert_eq!((row.target, row.values.as_slice()), (3, &[0][..]));
    // The neighborhood's records are not ported.
    assert!(matches!(
        trap(relationship(&mut fx, 26, [0, 0, 2, 0, 7, 0, 0, 0])),
        Trap::Unsupported(_)
    ));
    assert_eq!(
        code(relationship(&mut fx, 26, [0, 4, 0, 0, 7, 0, 0, 0])),
        Some(0x15)
    );
}

fn next(fx: &mut Fixture, mode: u8, local: u8) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::PARAMETERS, 0, 1),
            ins(
                31,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 0, 0, mode | 0x80, scope::TEMPS, local, 0],
            ),
        ]),
    );
    let reference = fx.lamp().temps[1];
    fx.run_with(LAMP, 4096, 0, &[reference, 0, 0, 0], BUDGET)
}

#[test]
fn set_to_next_walks_the_objects_on_a_tile_and_beside_one() {
    let (mut fx, _) = lot();
    let middle = stand(&mut fx, [2, 2]);
    let east = stand(&mut fx, [3, 2]);
    let behind = stand(&mut fx, [2, 3]);
    let south = stand(&mut fx, [2, 3]);
    let outside = stand(&mut fx, [1, 2]);
    fx.lamp_mut().temps = vec![0, middle as i16, 0, 0, 0, 0, 0, 0];
    // Mode 9: the first object on each tile beside the one local 0 names,
    // if it is in the same room. East, then south; the tile to the west
    // is outside the walls, and nothing stands to the north.
    let mut found = Vec::new();
    while next(&mut fx, 9, 0) == Ok(true) {
        found.push(fx.lamp().temps[0] as u16);
    }
    assert_eq!(found, [east, south]);
    assert_ne!(
        fx.world.objects[&outside].data[29],
        fx.world.objects[&middle].data[29]
    );
    // Mode 8: round the objects on the cursor's tile.
    fx.lamp_mut().temps[0] = south as i16;
    assert_eq!(next(&mut fx, 8, 0), Ok(true));
    assert_eq!(fx.lamp().temps[0], behind as i16);
    assert_eq!(next(&mut fx, 8, 0), Ok(true));
    assert_eq!(fx.lamp().temps[0], south as i16);
    // An object that stands nowhere is the false branch.
    fx.lamp_mut().temps[0] = LAMP as i16;
    assert_eq!(next(&mut fx, 8, 0), Ok(false));
}

#[test]
fn a_new_object_can_be_created_off_the_lot_and_run_a_tree_by_name() {
    let (mut fx, file) = widgets();
    let guid = WIDGET.to_le_bytes();
    // The widget's tree "Poke" adds the first temp to the widget's
    // attribute 2 and returns false.
    fx.add(
        file,
        4300,
        tree(vec![ins(
            2,
            RETURN_FALSE,
            RETURN_FALSE,
            expr(scope::STACK_ATTRIBUTES, 2, op::ADD, scope::PARAMETERS, 0),
        )]),
    );
    let names = &mut fx.program.libraries.get_mut(&file).unwrap().names;
    names.insert(4300, "Poke".to_owned());
    fx.program.libraries.get_mut(&GLOBAL).unwrap().tree_names =
        vec![String::new(), "poke".to_owned(), "absent".to_owned()];
    let by_name = |index: u8, on_true: u8, on_false: u8| {
        ins(28, on_true, on_false, [47, 1, 1, 0, index, 2, 0, 0])
    };
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(
                42,
                1,
                ERROR,
                [guid[0], guid[1], guid[2], guid[3], 6, 0, 0, 0],
            ),
            // The tree returns false, so the false branch goes on.
            by_name(2, ERROR, 2),
            // An empty name and a name no tree has are the false branch.
            by_name(1, ERROR, 3),
            by_name(3, ERROR, RETURN_TRUE),
        ]),
    );
    fx.lamp_mut().temps = vec![4, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(fx.run(), Ok(true));
    // The new object is registered at the head of the list, off the lot,
    // with its Init and first turn run.
    let id = fx.world.order[0];
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.guid, WIDGET);
    assert_eq!(widget.position, [-16, -16, 1]);
    assert_eq!(widget.attributes[0], 7);
    assert_eq!(widget.stack.len(), 2);
    // The named tree ran on the lamp's thread with the widget as its
    // stack object and the lamp's temps as its arguments.
    assert_eq!(widget.attributes[2], 4);
    assert!(fx.lamp().stack.is_empty());
    // Placing a new object is not ported.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            42,
            RETURN_TRUE,
            RETURN_FALSE,
            [guid[0], guid[1], guid[2], guid[3], 0, 0, 0, 0],
        )]),
    );
    let text = trap(fx.run()).to_string();
    assert!(text.contains("Create New Object Instance mode 0"), "{text}");
}
