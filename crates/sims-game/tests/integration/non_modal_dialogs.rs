//! Dialogs that do not stop the game (Dialog operand 7 bit 7 set), end to
//! end through [`Game`] on the open pack, with no install: the text the
//! HUD keeps for each record, a reshown dialog whose text changed, a
//! record that cannot be read, a text field's keys, and a co-op answer.
//!
//! No lot shows one while it idles, so each test puts the record in the
//! world as a caller's `Dialog` leaves it (save → patch → restore): a new
//! record, which the next tick shows (`0x436da0`), or a shown one made new
//! again, as its caller's next `Dialog` does (`0x437058`). The callers are
//! the open pack's idle objects, whose frames do not run in these ticks;
//! a caller's text comes from the file of its frame's code owner, so a
//! test points that at the phone's file (its strings), at another file
//! (other text for the same operands, as other `$Local` values give), or
//! at a type the lot does not have (a record that cannot be read).
//!
//! The tests compare the text shown between records, never with words:
//! they are the open pack's own.

use sims_game::coop::{CoopStartOptions, CoopTransport};
use sims_game::game::Game;
use sims_game::hud::DialogKey;
use sims_game::open_pack::discover_open_pack_dir;
use sims_net::{ManifestDigest, MemoryMesh, SessionConfig, Transport};
use sims_sim::{Dialog, World};
use sims_vm::boot::Frames;
use sims_vm::boot::save::Saved;

/// The open pack's wall phone, whose file holds the strings shown.
const PHONE: u32 = 0x7F00_0030;
/// Two more of the open pack's objects, idle while the tests run.
const LAMP: u32 = 0x7F00_0040;
const BASIN: u32 = 0x7F00_0010;
/// A code owner type the lot does not have: its caller cannot be read.
const NO_TYPE: i16 = i16::MAX;
/// The record statuses (`sims_vm` `primitives::dialog`).
const NEW: u8 = 0;
const SHOWN: u8 = 1;
const FIRST: u8 = 2;
/// Kinds (operand 5): a Yes/No choice and a text field.
const YES_NO: u8 = 1;
const TEXT: u8 = 3;
/// Two of the phone's strings (`STR#` 301 entries), of different text.
const ONE: u8 = 2;
const OTHER: u8 = 7;

/// The open pack in play mode, waiting for answers, after its first tick.
fn open() -> Game {
    let pack = discover_open_pack_dir().expect("the workspace's open-pack/");
    let mut game = Game::open_open_pack(&pack, &[], true, Frames::default()).expect("open pack");
    game.resize([800, 600]);
    assert_eq!(game.run(1), 1);
    game
}

/// The object of `guid` on the lot.
fn object(game: &Game, guid: u32) -> u16 {
    let world = game.lot().world();
    *world
        .objects
        .iter()
        .find(|(_, o)| o.guid == guid)
        .unwrap_or_else(|| panic!("no object {guid:#x}"))
        .0
}

/// The code owner type of the object of `guid` (its own type).
fn own_type(game: &Game, guid: u32) -> i16 {
    let id = object(game, guid);
    game.lot().world().objects[&id].stack[0].code_owner_type
}

/// A non-modal dialog record of `kind` from `caller`, its text the
/// caller's string `text`, as a caller's `Dialog` makes it.
fn record(caller: u16, kind: u8, text: u8) -> Dialog {
    Dialog {
        caller,
        operands: [0, 0, text, 0, 0, kind, 0, 0x80],
        status: NEW,
    }
}

/// Changes the lot's world as `edit` does, through a save and a restore.
fn patch(game: &mut Game, edit: impl FnOnce(&mut World)) {
    let lot = game.lot_mut();
    let saved = lot.save().expect("save");
    let body = saved.world().encode();
    let encoded = saved.encode();
    let tail = encoded[8..]
        .strip_prefix(body.as_slice())
        .expect("the world comes first");
    let mut world = saved.world().clone();
    edit(&mut world);
    let mut rebuilt = encoded[..8].to_vec();
    rebuilt.extend_from_slice(&world.encode());
    rebuilt.extend_from_slice(tail);
    lot.restore(Saved::decode(&rebuilt).expect("decode"))
        .expect("restore");
}

/// Points `caller`'s frame at the code of type `owner`: its dialogs read
/// that file's strings.
fn set_owner(world: &mut World, caller: u16, owner: i16) {
    world.objects.get_mut(&caller).unwrap().stack[0].code_owner_type = owner;
}

/// The status of `caller`'s record.
fn status(game: &Game, caller: u16) -> u8 {
    game.lot()
        .world()
        .dialogs
        .records
        .iter()
        .find(|r| r.caller == caller)
        .expect("the record")
        .status
}

/// Makes `caller`'s shown record new again, as its caller's next `Dialog`
/// does, and runs the tick that shows it.
fn reshow(game: &mut Game, caller: u16) {
    patch(game, |world| {
        for r in &mut world.dialogs.records {
            if r.caller == caller {
                r.status = NEW;
            }
        }
    });
    assert_eq!(game.run(1), 1);
    assert_eq!(status(game, caller), SHOWN);
}

/// The dialog drawn: its caller and its lines.
fn drawn(game: &mut Game) -> Option<(u16, Vec<String>)> {
    game.render();
    game.dialog()
        .map(|l| (l.caller, l.lines.iter().map(|(s, _)| s.clone()).collect()))
}

/// The middle of the drawn dialog's dismiss box.
fn dismiss(game: &mut Game) -> [i32; 2] {
    game.render();
    let b = game.dialog().expect("a dialog drawn").buttons[0];
    [b.x + b.width / 2, b.y + b.height / 2]
}

#[test]
fn the_hud_keeps_each_records_text_and_passes_over_one_it_cannot_read() {
    let mut game = open();
    let phone = object(&game, PHONE);
    let lamp = object(&game, LAMP);
    let basin = object(&game, BASIN);
    let phone_type = own_type(&game, PHONE);
    // First in the registry, the basin's record, which cannot be read; then
    // the phone's and the lamp's, both read from the phone's file.
    patch(&mut game, |world| {
        set_owner(world, basin, NO_TYPE);
        set_owner(world, lamp, phone_type);
        world.dialogs.records.push(record(basin, YES_NO, ONE));
        world.dialogs.records.push(record(phone, YES_NO, ONE));
        world.dialogs.records.push(record(lamp, YES_NO, OTHER));
    });
    assert_eq!(game.run(1), 1, "the game goes on");
    for caller in [basin, phone, lamp] {
        assert_eq!(status(&game, caller), SHOWN);
    }
    // The one that cannot be read does not hide the others.
    let (caller, phone_text) = drawn(&mut game).expect("a dialog drawn");
    assert_eq!(caller, phone);
    assert!(!phone_text.is_empty());
    assert!(!game.has_non_modal_dialog_entry());

    // The lamp's text was read when it was shown: once its frame can no
    // longer be read, the text kept is what shows.
    patch(&mut game, |world| set_owner(world, lamp, NO_TYPE));
    game.answer(phone, 0);
    assert_eq!(game.note(), None);
    assert_eq!(status(&game, phone), FIRST, "the first button's answer");
    let (caller, lamp_text) = drawn(&mut game).expect("the lamp's dialog drawn");
    assert_eq!(caller, lamp);
    assert!(!lamp_text.is_empty());
    assert_ne!(lamp_text, phone_text, "its own text, not the phone's");
    // Shown again unread, it keeps that text.
    reshow(&mut game, lamp);
    assert_eq!(drawn(&mut game), Some((lamp, lamp_text)));

    // Once the lamp's is answered only the basin's is left, which was never
    // read: nothing is drawn, though the record stays shown.
    game.answer(lamp, 1);
    assert_eq!(status(&game, lamp), FIRST + 1, "the second button's answer");
    assert_eq!(drawn(&mut game), None);
    assert_eq!(status(&game, basin), SHOWN);
    // Readable again, it is read and drawn.
    patch(&mut game, |world| set_owner(world, basin, phone_type));
    assert_eq!(drawn(&mut game), Some((basin, phone_text)));
}

#[test]
fn a_reshown_dialog_with_other_text_replaces_the_one_shown_and_its_press() {
    let mut game = open();
    let phone = object(&game, PHONE);
    let phone_type = own_type(&game, PHONE);
    let lamp_type = own_type(&game, LAMP);
    patch(&mut game, |world| {
        world.dialogs.records.push(record(phone, YES_NO, ONE))
    });
    assert_eq!(game.run(1), 1);
    let (_, text) = drawn(&mut game).expect("the phone's dialog drawn");

    // Shown again with the same text, the widget is the same one: a press
    // held on it answers on release.
    let point = dismiss(&mut game);
    game.press(point);
    reshow(&mut game, phone);
    assert_eq!(drawn(&mut game), Some((phone, text.clone())));
    game.release(point);
    assert_eq!(status(&game, phone), FIRST, "the press answered");

    // Shown again with other text (the same instruction, other `$Local`
    // values), it replaces the one shown with fresh input state: a press
    // held on the old text answers nothing.
    reshow(&mut game, phone);
    assert_eq!(drawn(&mut game), Some((phone, text.clone())));
    let point = dismiss(&mut game);
    game.press(point);
    patch(&mut game, |world| set_owner(world, phone, lamp_type));
    reshow(&mut game, phone);
    let (caller, other) = drawn(&mut game).expect("the new text drawn");
    assert_eq!(caller, phone);
    assert_ne!(other, text);
    game.release(point);
    assert_eq!(
        status(&game, phone),
        SHOWN,
        "the old press answered nothing"
    );
    // A press on the new one answers it.
    patch(&mut game, |world| set_owner(world, phone, phone_type));
    let point = dismiss(&mut game);
    game.press(point);
    game.release(point);
    assert_eq!(status(&game, phone), FIRST);
    assert_eq!(drawn(&mut game), None);
}

#[test]
fn a_non_modal_text_field_takes_the_keys_after_a_modal_dialog() {
    let mut game = open();
    let phone = object(&game, PHONE);
    let lamp = object(&game, LAMP);
    let phone_type = own_type(&game, PHONE);
    patch(&mut game, |world| {
        world.dialogs.records.push(record(phone, TEXT, ONE))
    });
    assert_eq!(game.run(1), 1);
    assert!(drawn(&mut game).is_some_and(|(caller, _)| caller == phone));
    assert!(game.has_non_modal_dialog_entry());
    let start = game.dialog_entry().expect("a field").to_owned();

    // A modal dialog in front (a choice: a one-button one answers itself)
    // has the keys: Enter answers it, not the field behind it.
    patch(&mut game, |world| {
        set_owner(world, lamp, phone_type);
        let mut modal = record(lamp, YES_NO, OTHER);
        modal.operands[7] = 0;
        world.dialogs.records.push(modal);
    });
    assert_eq!(game.run(1), 0, "the modal dialog stops the game");
    assert!(game.asking().is_some());
    assert!(drawn(&mut game).is_some_and(|(caller, _)| caller == lamp));
    assert!(!game.has_non_modal_dialog_entry());
    assert!(!game.type_dialog_char('x'), "the modal dialog has no field");
    let _ = game.dialog_submit();
    assert!(game.asking().is_none());
    assert_eq!(
        status(&game, lamp),
        FIRST,
        "Enter answered the modal dialog"
    );
    assert_eq!(status(&game, phone), SHOWN);

    // Back on the field: characters, Backspace, a reshow of the same text
    // keeping what was typed, and Escape doing nothing to a one-button
    // field, as on a modal one.
    assert!(drawn(&mut game).is_some_and(|(caller, _)| caller == phone));
    assert!(game.has_non_modal_dialog_entry());
    assert!(game.dialog_end(false, false));
    for _ in start.chars() {
        assert!(game.dialog_backspace());
    }
    for ch in "Abc".chars() {
        assert!(game.type_dialog_char(ch));
    }
    assert!(game.dialog_backspace());
    assert!(game.type_dialog_char('e'));
    assert_eq!(game.dialog_entry(), Some("Abe"));
    reshow(&mut game, phone);
    drawn(&mut game);
    assert_eq!(game.dialog_entry(), Some("Abe"), "a reshow keeps the text");
    let _ = game.dialog_confirm_key(DialogKey::Escape);
    assert_eq!(status(&game, phone), SHOWN);
    assert_eq!(game.dialog_entry(), Some("Abe"));

    // Shift+Left twice selects "be" (`0x5f2ec7`); a paste replaces it, its
    // tab widened to four spaces (`0x5d7cf1`); Ctrl+Shift+Left selects the
    // word before the caret (`0x5f3188`) and a cut takes it out.
    // The game keeps its own clipboard, so the test leaves the system's
    // alone.
    game.use_local_clipboard();
    assert!(!game.dialog_paste(None), "nothing on the clipboard yet");
    assert!(game.dialog_move_left(false, true));
    assert!(game.dialog_move_left(false, true));
    assert!(game.dialog_paste(Some("x\ty")));
    assert_eq!(game.dialog_entry(), Some("Ax    y"));
    assert!(game.dialog_move_left(true, true));
    assert!(game.dialog_cut().is_some_and(|t| t == "y"));
    assert_eq!(game.dialog_entry(), Some("Ax    "));
    // What was cut pastes back.
    assert!(game.dialog_paste(None));
    assert_eq!(game.dialog_entry(), Some("Ax    y"));
    assert!(game.dialog_move_left(true, true));
    assert!(game.dialog_cut().is_some_and(|t| t == "y"));
    // Insert switches to overwriting: Home, then two characters replace
    // the first two.
    assert!(game.dialog_toggle_overwrite());
    assert!(game.dialog_home(false, false));
    assert!(game.type_dialog_char('A'));
    assert!(game.type_dialog_char('b'));
    assert!(game.dialog_toggle_overwrite());
    assert!(game.dialog_end(false, true));
    assert!(game.dialog_backspace());
    assert!(game.type_dialog_char('e'));
    assert_eq!(game.dialog_entry(), Some("Abe"));

    // A double click selects the word only when its press went to the
    // field (`0x5f2c21`), not when another window took it.
    let entry = game.dialog().and_then(|l| l.entry).expect("the field");
    let point = [entry.x + 2, entry.y + entry.height / 2];
    assert!(!game.dialog_double_click(point), "no press in the field");
    game.press(point);
    assert!(game.dialog_double_click(point));
    game.release(point);
    assert_eq!(game.dialog_entry(), Some("Abe"));

    // Enter answers with the first button and the text typed, which the
    // caller's next `Dialog` reads.
    let _ = game.dialog_submit();
    assert_eq!(status(&game, phone), FIRST);
    assert_eq!(game.last_dialog_text(), "Abe");
    assert_eq!(drawn(&mut game), None);
    assert!(!game.has_non_modal_dialog_entry());
    assert!(!game.type_dialog_char('x'), "no field takes keys now");
}

/// Delivers the mesh's messages and lets each game handle them.
fn pump(mesh: &MemoryMesh, host: &mut Game, peer: &mut Game) {
    for _ in 0..2 {
        let _ = mesh.step_n(4);
        host.coop_pump(true, true).expect("host pump");
        let _ = mesh.step_n(4);
        peer.coop_pump(false, true).expect("peer pump");
    }
}

/// A host and a peer on a memory mesh, the host's world holding new
/// non-modal records from the phone and the lamp (read from the phone's
/// file), which co-op shows whatever the setting. With `prime`, the host's
/// prime tick at the start shows them; without, the first lockstep tick.
/// Returns once the peer has the host's snapshot.
fn co_op(prime: bool) -> (MemoryMesh, Game, Game, u16, u16) {
    let mut host = open();
    let mut peer = open();
    let phone = object(&host, PHONE);
    let lamp = object(&host, LAMP);
    let phone_type = own_type(&host, PHONE);
    patch(&mut host, |world| {
        set_owner(world, lamp, phone_type);
        world.dialogs.records.push(record(phone, YES_NO, ONE));
        world.dialogs.records.push(record(lamp, YES_NO, OTHER));
    });
    let mesh = MemoryMesh::new(7);
    let host_tx = mesh.add_node("mem://host");
    let host_addr = host_tx.local_address().clone();
    let peer_tx = mesh.add_node("mem://peer");
    let manifest = ManifestDigest([0x3C; 32]);
    host.coop_start_host_with_transport(
        CoopTransport::memory(host_tx),
        manifest,
        CoopStartOptions {
            player_name: "Host".into(),
            session_config: SessionConfig {
                fingerprint_interval: 1,
                ..SessionConfig::default()
            },
            prime_tick_on_start: prime,
            ..CoopStartOptions::default()
        },
    )
    .expect("host start");
    if prime {
        // The host read them from the prime tick, before any frame is
        // drawn, and the phone's timer runs.
        assert_eq!(status(&host, phone), SHOWN);
        assert!(host.auto_answer_running());
    } else {
        assert_eq!(status(&host, phone), NEW);
    }
    peer.coop_start_join_with_transport(
        CoopTransport::memory(peer_tx),
        host_addr,
        manifest,
        "Peer",
    )
    .expect("peer join");
    for _ in 0..40 {
        pump(&mesh, &mut host, &mut peer);
        if peer.coop_controller().snapshot_restore_count() >= 1 {
            break;
        }
    }
    assert_eq!(peer.coop_controller().snapshot_restore_count(), 1);
    for game in [&host, &peer] {
        assert_eq!(status(game, phone), SHOWN);
        assert_eq!(status(game, lamp), SHOWN);
    }
    (mesh, host, peer, phone, lamp)
}

#[test]
fn a_host_reads_the_non_modal_dialogs_its_prime_tick_showed() {
    let (_mesh, mut host, mut peer, phone, _) = co_op(true);
    assert!(host.auto_answer_running());
    let drawn_host = drawn(&mut host).expect("the host draws it");
    assert_eq!(drawn_host.0, phone);
    assert_eq!(drawn(&mut peer), Some(drawn_host));
}

#[test]
fn a_co_op_answer_closes_a_non_modal_dialog_on_every_peer() {
    let (mesh, mut host, mut peer, phone, lamp) = co_op(false);
    // The host read them from the batch that showed them, before any frame
    // is drawn, and its timer runs: in co-op a non-modal dialog's does.
    assert!(host.auto_answer_running());
    let host_drawn = drawn(&mut host).expect("the host draws it");
    assert_eq!(host_drawn.0, phone);
    assert_eq!(drawn(&mut peer), Some(host_drawn.clone()));
    assert!(peer.auto_answer_running());

    // The peer answers the phone's with its second button: a command,
    // applied on both between the same two ticks.
    let note = peer.note().map(str::to_owned);
    peer.answer(phone, 1);
    assert_eq!(peer.note(), note.as_deref(), "the answer was sent");
    assert_eq!(status(&peer, phone), SHOWN, "not until the command runs");
    for _ in 0..20 {
        pump(&mesh, &mut host, &mut peer);
        if status(&host, phone) != SHOWN && status(&peer, phone) != SHOWN {
            break;
        }
    }
    assert_eq!(status(&host, phone), FIRST + 1);
    assert_eq!(status(&peer, phone), FIRST + 1);
    assert_eq!(host.lot().coop_world_hash(), peer.lot().coop_world_hash());
    // Both go on to the lamp's (the peer's drawn to compare).
    let (caller, lamp_text) = drawn(&mut peer).expect("the lamp's drawn");
    assert_eq!(caller, lamp);
    assert_ne!(lamp_text, host_drawn.1);

    // The host took up the lamp's when the answer ran, with no frame drawn
    // since: its timer runs out and sends the answer as a press would, the
    // same on both.
    host.advance(40_000_001);
    for _ in 0..20 {
        pump(&mesh, &mut host, &mut peer);
        if status(&host, lamp) != SHOWN && status(&peer, lamp) != SHOWN {
            break;
        }
    }
    let answered = status(&host, lamp);
    assert!(answered > SHOWN, "the timer answered");
    assert_eq!(status(&peer, lamp), answered);
    assert_eq!(host.lot().coop_world_hash(), peer.lot().coop_world_hash());
    assert_eq!(drawn(&mut host), None);
    assert_eq!(drawn(&mut peer), None);
}
