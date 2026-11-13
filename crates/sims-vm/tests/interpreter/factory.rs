//! The object factory: storage, registration, Init, the first turn and the
//! finishing touches.

use crate::support::*;
use sims_formats::behavior::{Interaction, InteractionTable};
use sims_vm::Trap;

pub const WIDGET: u32 = 0x3333;
/// The widget's Init and Main trees.
const INIT: u16 = 4096;
const MAIN: u16 = 4097;
/// The global tree that sleeps.
const SLEEP: u16 = 256;

/// A fixture with a widget type: Init sets attribute 0 to 7, and Main
/// sleeps 20 turns over and over.
pub fn widgets() -> (Fixture, usize) {
    let mut fx = Fixture::new();
    let file = fx.add_type(WIDGET, 4, INIT, MAIN);
    fx.add(GLOBAL, SLEEP, sleeper());
    fx.add(
        file,
        INIT,
        tree(vec![assign(
            scope::MY_ATTRIBUTES,
            0,
            scope::LITERAL,
            7,
            RETURN_TRUE,
        )]),
    );
    fx.add(
        file,
        MAIN,
        tree(vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
    );
    (fx, file)
}

fn create(fx: &mut Fixture, guid: u32) -> Result<u16, sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.create(&mut fx.world, guid)
}

#[test]
fn a_new_object_is_built_registered_and_run() {
    let (mut fx, file) = widgets();
    let words = fx.words(file);
    words[18] = 250; // price
    words[20] = 5; // SLOT chunk
    words[49] = 9; // copied to data word 58
    words[58] = 12; // attributes
    words[65] = 2; // dynamic sprites
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .containers
        .insert(5, 2);
    assert_eq!(create(&mut fx, WIDGET), Ok(3));
    // New objects go to the head of the registration list.
    assert_eq!(fx.world.order, [3, LAMP, SIM]);
    let widget = &fx.world.objects[&3];
    assert_eq!((widget.id, widget.guid), (3, WIDGET));
    assert_eq!(widget.position, [-16, -16, 1], "off the lot");
    // Init ran after the reset cleared the attributes.
    assert_eq!(widget.attributes, [7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(widget.temps, [0; 8]);
    assert_eq!(widget.dynamic_sprites, [0, 0]);
    // The tile chain's slot and one for each container.
    assert_eq!(widget.slots.len(), 3);
    let mut data = [0; 72];
    data[3] = -1; // no slot
    data[11] = 3; // its ID
    data[14] = -1;
    data[29] = -5; // no room
    data[41] = 250;
    data[58] = 9;
    data[67] = 2;
    // The lot's hour, minute, year, month and day (globals 0, 5, 8, 7, 1).
    (data[24], data[21]) = (100, 105);
    (data[45], data[46], data[47]) = (108, 107, 101);
    assert_eq!(widget.data, data);
    // Main is the bottom frame, with four zero arguments, and it called
    // the sleeping tree on the object's first turn.
    let frames: Vec<_> = widget
        .stack
        .iter()
        .map(|f| (f.tree, f.node, f.stack_object, &f.parameters[..]))
        .collect();
    assert_eq!(
        frames,
        [(MAIN, 0, 0, &[0, 0, 0, 0][..]), (SLEEP, 0, 0, &[20][..])]
    );
    // The scheduler counts the Sleep down, and the object is marked ready.
    assert_eq!(widget.countdown(), Some(19));
    assert_eq!(widget.flags(), 1);
    assert!(widget.last_result, "Init returned true");
    assert!(!widget.removal_skips_resets);
    // The lot's type table gets the type, under the lowest unused ID, and
    // the frames name it.
    assert_eq!(fx.world.types[&1], WIDGET);
    assert!(widget.stack.iter().all(|f| f.code_owner_type == 1));
}

#[test]
fn ids_are_the_lowest_unused() {
    let (mut fx, _) = widgets();
    fx.world.objects.remove(&LAMP);
    fx.world.order.retain(|&id| id != LAMP);
    assert_eq!(create(&mut fx, WIDGET), Ok(1));
    assert_eq!(create(&mut fx, WIDGET), Ok(3));
    assert_eq!(fx.world.order, [3, 1, SIM]);
    // One type table entry serves both.
    assert_eq!(fx.world.types.values().filter(|&&g| g == WIDGET).count(), 1);
}

#[test]
fn a_definition_gets_at_least_eight_attributes() {
    for count in [0, 3, -5] {
        let (mut fx, file) = widgets();
        fx.words(file)[58] = count;
        let id = create(&mut fx, WIDGET).unwrap();
        assert_eq!(fx.world.objects[&id].attributes.len(), 8);
    }
}

#[test]
fn init_runs_synchronously_above_main() {
    // Init's result does not matter.
    let (mut fx, file) = widgets();
    fx.add(
        file,
        INIT,
        tree(vec![assign(
            scope::MY_ATTRIBUTES,
            1,
            scope::LITERAL,
            4,
            RETURN_FALSE,
        )]),
    );
    let id = create(&mut fx, WIDGET).unwrap();
    assert_eq!(fx.world.objects[&id].attributes[1], 4);
    assert_eq!(fx.world.objects[&id].stack.len(), 2);
    // Init sees Main below it: a wait in Init is an error, and the object
    // is left as it was for inspection.
    let (mut fx, file) = widgets();
    fx.add(
        file,
        INIT,
        tree(vec![ins(
            SLEEP,
            RETURN_TRUE,
            RETURN_TRUE,
            [3, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    let fault = create(&mut fx, WIDGET).unwrap_err();
    assert_eq!((fault.object, fault.trap.code()), (3, Some(0x44f)));
    let trees: Vec<_> = fx.world.objects[&3].stack.iter().map(|f| f.tree).collect();
    assert_eq!(trees, [MAIN, INIT, SLEEP]);
}

#[test]
fn a_missing_init_is_skipped() {
    // No Init named, and an Init whose tree does not exist.
    for init in [0, 4500] {
        let mut fx = Fixture::new();
        let file = fx.add_type(WIDGET, 4, init, MAIN);
        fx.add(GLOBAL, SLEEP, sleeper());
        fx.add(
            file,
            MAIN,
            tree(vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
        );
        let id = create(&mut fx, WIDGET).unwrap();
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.attributes, [0; 8]);
        assert_eq!(widget.countdown(), Some(19));
        assert!(!widget.last_result);
    }
}

#[test]
fn main_may_return_on_its_first_turn() {
    let (mut fx, file) = widgets();
    fx.add(
        file,
        MAIN,
        tree(vec![assign(
            scope::MY_ATTRIBUTES,
            2,
            scope::LITERAL,
            5,
            RETURN_TRUE,
        )]),
    );
    let id = create(&mut fx, WIDGET).unwrap();
    let widget = &fx.world.objects[&id];
    assert!(widget.stack.is_empty());
    assert_eq!(widget.attributes[2], 5);
    assert_eq!((widget.countdown(), widget.flags()), (Some(-1), 1));
}

#[test]
fn the_lockout_word_counts_down_at_the_end_of_the_turn() {
    let (mut fx, file) = widgets();
    fx.add(
        file,
        INIT,
        tree(vec![assign(
            scope::MY_DATA,
            25,
            scope::LITERAL,
            2,
            RETURN_TRUE,
        )]),
    );
    let id = create(&mut fx, WIDGET).unwrap();
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.data[25], 1);
    // Locked out, so the Sleep counted by itself: the scheduler was not
    // given the count. Init touching word 25 also woke the object.
    assert_eq!(widget.stack[1].parameters, [19]);
    assert_eq!(widget.countdown(), Some(-2));
}

fn interaction(autonomy: u32, flags: u32) -> Interaction {
    Interaction {
        action: 4100,
        test: 0,
        string_index: 0,
        flags,
        autonomy,
        attenuation_code: 0,
        attenuation: 0.0,
        joining_index: -1,
        motives: Vec::new(),
        flags2: 0,
    }
}

/// The widget's scheduler flags when its definition names table 9 and the
/// file holds `table` under that ID.
fn flags_with(table: Vec<Interaction>) -> u16 {
    let (mut fx, file) = widgets();
    fx.words(file)[7] = 9;
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .interactions
        .insert(
            9,
            Some(InteractionTable {
                version: 8,
                interactions: table,
            }),
        );
    let id = create(&mut fx, WIDGET).unwrap();
    fx.world.objects[&id].flags()
}

#[test]
fn the_flags_say_whether_the_object_advertises() {
    // Ready is flag 1, advertising flag 8.
    assert_eq!(flags_with(vec![]), 1);
    assert_eq!(flags_with(vec![interaction(50, 0)]), 9);
    // A threshold of 100 or more does not advertise; the comparison is
    // signed.
    assert_eq!(flags_with(vec![interaction(100, 0)]), 1);
    assert_eq!(flags_with(vec![interaction(u32::MAX, 0)]), 9);
    // Flag bit 7 keeps an interaction from being advertised.
    assert_eq!(flags_with(vec![interaction(50, 128)]), 1);
    assert_eq!(
        flags_with(vec![interaction(50, 128), interaction(99, 1)]),
        9
    );
    // No table under the ID the definition names.
    let (mut fx, file) = widgets();
    fx.words(file)[7] = 9;
    let id = create(&mut fx, WIDGET).unwrap();
    assert_eq!(fx.world.objects[&id].flags(), 1);
}

#[test]
fn a_table_the_reader_cannot_decode_is_an_error_when_used() {
    let (mut fx, file) = widgets();
    fx.words(file)[7] = 9;
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .interactions
        .insert(9, None);
    let fault = create(&mut fx, WIDGET).unwrap_err();
    assert!(fault.to_string().contains("TTAB #9"), "{fault}");
}

#[test]
fn data_word_60_sets_the_queued_flag() {
    let (mut fx, file) = widgets();
    fx.add(
        file,
        INIT,
        tree(vec![assign(
            scope::MY_DATA,
            60,
            scope::LITERAL,
            1,
            RETURN_TRUE,
        )]),
    );
    let id = create(&mut fx, WIDGET).unwrap();
    assert_eq!(fx.world.objects[&id].flags(), 3);
}

#[test]
fn a_type_with_an_instance_list_gets_the_new_instance() {
    let (mut fx, _) = widgets();
    fx.world.instances.insert(WIDGET, vec![40]);
    fx.world.instances.insert(LAMP_GUID, vec![LAMP]);
    let id = create(&mut fx, WIDGET).unwrap();
    assert_eq!(fx.world.instances[&WIDGET], [40, id]);
    assert_eq!(fx.world.instances[&LAMP_GUID], [LAMP]);
    // A type nothing has asked about gets no list.
    let (mut fx, _) = widgets();
    create(&mut fx, WIDGET).unwrap();
    assert!(fx.world.instances.is_empty());
}

#[test]
fn types_with_factories_of_their_own_are_refused() {
    let unsupported = |change: fn(&mut [i16; sims_vm::DEFINITION_WORDS])| {
        let (mut fx, file) = widgets();
        change(fx.words(file));
        let fault = create(&mut fx, WIDGET).unwrap_err();
        assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
        // Nothing was registered.
        assert_eq!(fx.world.order, [LAMP, SIM]);
        assert_eq!(fx.world.objects.len(), 2);
        fault.to_string()
    };
    // A person, the two cursor types, and a tile of a multi-tile object.
    for kind in [2, 8, 9] {
        let text = match kind {
            2 => unsupported(|w| w[9] = 2),
            8 => unsupported(|w| w[9] = 8),
            _ => unsupported(|w| w[9] = 9),
        };
        assert!(text.contains(&format!("object type {kind}")), "{text}");
    }
    assert!(unsupported(|w| w[10] = 3).contains("multi-tile"));
    // A GUID with no definition.
    let (mut fx, _) = widgets();
    assert!(matches!(
        create(&mut fx, 0x7777).unwrap_err().trap,
        Trap::Invalid(_)
    ));
}

#[test]
fn a_type_needs_a_main_tree_that_takes_at_most_four_arguments() {
    let mut fx = Fixture::new();
    fx.add_type(WIDGET, 4, 0, 0);
    let fault = create(&mut fx, WIDGET).unwrap_err();
    assert!(fault.to_string().contains("no Main tree"), "{fault}");
    let (mut fx, file) = widgets();
    fx.add(
        file,
        MAIN,
        shaped(
            0x8002,
            0,
            5,
            0,
            vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])],
        ),
    );
    let fault = create(&mut fx, WIDGET).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
}
