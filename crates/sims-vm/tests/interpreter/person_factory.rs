//! The person factory (the person path of 0x440b2d): a script makes a new
//! person with Create New Object Instance mode 8, or the factory is asked
//! for one directly.

use crate::support::*;
use sims_content::{Profile, Skeleton};
use sims_core::CrtRng;
use sims_formats::tuning::{Constant, Constants};
use sims_sim::neighbors::Neighbor;
use sims_vm::{Interpreter, Trap};

const ADULT: u32 = 0x6001;
const DOG: u32 = 0x6002;
const MAIN: u16 = 4097;

/// A lot with the lamp and the Sim (IDs 1 and 2), and two person types
/// [`equip`]ped with records whose words are all `record_word`: an adult
/// and a dog. The adult's Main sets its first argument to 100 and sleeps
/// on it.
fn lot(record_word: i16) -> Fixture {
    let mut fx = Fixture::new();
    let file = fx.add_type(ADULT, 2, 0, MAIN);
    fx.words(file)[word::PRICE] = 250;
    fx.add(
        file,
        MAIN,
        tree(vec![
            assign(scope::PARAMETERS, 0, scope::LITERAL, 100, 1),
            ins(0, 0, 0, [0; 8]),
        ]),
    );
    let dog = fx.add_type(DOG, 2, 0, MAIN);
    fx.add(dog, MAIN, tree(vec![ins(0, 0, 0, [0; 8])]));
    equip(&mut fx, ADULT, "adult", record_word);
    equip(&mut fx, DOG, "Dog", record_word);
    fx.world.crt_rng = CrtRng::from_state(77);
    fx
}

/// Gives a fixture what making a person of type `guid` needs beyond its
/// definition: a place in the lot's type table, a neighborhood record of
/// revision 7 whose words are all `record_word`, a skeleton named `name`,
/// an empty profile, and the global file's motive tuning.
pub fn equip(fx: &mut Fixture, guid: u32, name: &str, record_word: i16) {
    let next = fx.world.types.keys().max().copied().unwrap_or(0) + 1;
    fx.world.types.insert(next, guid);
    let id = 40 + fx.world.neighbors.slots.len() as i16;
    fx.world.neighbors.slots.push(Some(Neighbor {
        id,
        guid,
        revision: 7,
        data: vec![record_word; 256],
        relationships: Vec::new(),
        friends: 0,
        stale: false,
    }));
    fx.program.people.skeletons.insert(
        guid,
        Skeleton {
            name: name.into(),
            file: format!("{name}.bcf"),
            place: 0,
            scales: vec![vec![1]],
        },
    );
    fx.program.people.profiles.insert(
        guid,
        Profile {
            gender: None,
            age: None,
            skin: None,
            tones: Vec::new(),
        },
    );
    let global = fx.program.libraries.get_mut(&GLOBAL).unwrap();
    for id in [1, 5, 6] {
        global.tuning.insert(
            id,
            Constants {
                version: 0,
                entries: Vec::<Constant>::new(),
            },
        );
    }
    for id in [502, 504] {
        global
            .strings
            .insert(id, vec!["(-100;1)(100;1)".to_owned(); 7]);
    }
}

mod word {
    /// The definition word that holds the price.
    pub const PRICE: usize = 18;
}

fn create(fx: &mut Fixture, guid: u32) -> Result<u16, sims_vm::Fault> {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.create_person(&mut fx.world, guid)
}

#[test]
fn a_new_person_gets_the_lowest_unused_id_and_runs_its_init_and_first_update() {
    let mut fx = lot(11);
    let start = fx.world.crt_rng;
    let id = create(&mut fx, ADULT).unwrap();
    assert_eq!(id, 3);
    // Registered at the head of the list.
    assert_eq!(fx.world.order[0], 3);
    let object = &fx.world.objects[&3];
    assert_eq!(object.guid, ADULT);
    // The price, and the scheduler's ready flag.
    assert_eq!(object.data[41], 250);
    // Its thread is Main, which sleeps.
    assert_eq!(
        object
            .stack
            .iter()
            .map(|f| (f.tree, f.node))
            .collect::<Vec<_>>(),
        [(MAIN, 1)]
    );
    let person = object.person.as_ref().unwrap();
    // The record's ID; on a lot that is not residential, a visitor.
    assert_eq!(person.data[31], 40);
    assert_eq!(person.data[32], 1);
    // The record's interests, 11, were copied when the person was
    // attached to it after its Init.
    assert_eq!(person.data[46..54], [11; 8]);
    assert_ne!(fx.world.crt_rng, start);
    assert!(fx.world.skeleton_files.contains("adult.bcf"));
    assert!(fx.world.animators.contains_key(&3));
    let drawn = fx.world.crt_rng;
    // With interests of 10 or less in the record they stay low, and
    // default interests and personality are drawn: the stream goes
    // further.
    let mut fx = lot(10);
    create(&mut fx, ADULT).unwrap();
    assert_ne!(fx.world.crt_rng, drawn);
    let mut stream = drawn;
    let further = (0..1000).any(|_| {
        stream.next_u15();
        stream == fx.world.crt_rng
    });
    assert!(further);
}

#[test]
fn what_the_person_factory_does_not_port_fails_loudly() {
    // A pet: whether the factory makes it depends on the
    // installed-expansions mask.
    let mut fx = lot(11);
    let fault = create(&mut fx, DOG).unwrap_err();
    assert!(fault.to_string().contains("pet"), "{fault}");
    assert_eq!(fx.world.objects.len(), 2);
    // No skeleton.
    let mut fx = lot(11);
    fx.program.people.skeletons.remove(&ADULT);
    let fault = create(&mut fx, ADULT).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(ref m) if m.contains("skeleton")));
    // No neighborhood record: the original makes one.
    let mut fx = lot(11);
    fx.world.neighbors.slots[0] = None;
    assert!(create(&mut fx, ADULT).is_err());
    // No profile.
    let mut fx = lot(11);
    fx.program.people.profiles.remove(&ADULT);
    let fault = create(&mut fx, ADULT).unwrap_err();
    assert!(fault.to_string().contains("profile"), "{fault}");
}
