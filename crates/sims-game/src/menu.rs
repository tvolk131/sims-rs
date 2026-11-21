//! The pie menu: the interactions of an object the player clicked, for
//! the selected Sim to do.
//!
//! [`entries`] lists what the original's pie menu offers
//! ([`sims_vm::Interpreter::pie_menu`], 0x48d392): each entry of the
//! object's interaction table whose check passes for that Sim, in order.
//! Each is named by the name its check tree gave it (Add/Change Action
//! String, 0x458789), as it is; or else by the TTAs strings with the ID
//! of the TTAB it comes from, by its menu number, with "*" before it
//! when the entry has flag 0x80, as the original names it (0x41f167). An entry with no string is named by
//! its number; a person's TTAs names only its first entry (Talk), so a
//! person's menu is mostly numbers, as where the original reads the rest
//! is not found yet.
//!
//! **Listed on the lot.** Listing the menu runs the entries' check
//! trees, which change the lot as the original's do and may draw random
//! numbers, so [`entries`] lists it with `Lot::pie_menu` on the lot
//! itself, and the game calls it when the pointer first reaches an
//! object (0x48c834), as the original does, not on the click. Choosing an
//! entry queues the very action offered (`Lot::pick`).
//!
//! Where the simulation cannot list a menu yet (it fails loudly on what
//! it has not ported: an object that offers nothing; a version 0 table; a pet; an
//! unported scope or primitive), the lot is left as it was and the menu
//! falls back to every entry of the table without checks
//! (`every_entry`); [`Listing::unchecked`] says why, and choosing one
//! queues it by its menu number (`Lot::queue_interaction`). Any other
//! failure (a person who is not one, a check tree taking its error branch
//! or running out of instructions, a halted lot) is an error: no menu is
//! shown. The menu is shown as the original's pie (`sims_ui::pie`), at
//! the pointer.
//!
//! Which TTAB an object's entries come from, and which tile of a
//! multi-tile object they go to, the simulation decides in code it keeps
//! private ([`sims_vm::Program::interactions`], `interaction_target`);
//! `table` and [`target`] repeat that from the program's public data, for
//! the labels and the fallback.

use sims_formats::behavior::InteractionTable;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::saved_objects::Interaction;
use sims_formats::strings::StringTable;
use sims_render::Assets;
use sims_sim::World;
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::pie::{self, Pie, Pressed};
use sims_vm::DEFINITION_WORDS;
use sims_vm::boot::Lot;
use sims_vm::boot::OrderError;
use sims_vm::orders::MenuEntry;
use sims_vm::{SoundEvent, Trap};
use std::collections::BTreeMap;

/// The language the labels are read in (US English).
const LANGUAGE: u8 = 1;

/// The join word (+0) of the offer to join a running interaction
/// (0x41ee99).
const JOINS: i32 = 1;

/// The entry flag that puts "*" before the entry's name.
const DEBUG: u32 = 0x80;

/// One line of the menu.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The menu number the simulation queues by (the entry's string
    /// index).
    pub menu: u32,
    pub label: String,
    /// The action the simulation offered, which choosing the entry
    /// queues; None for the fallback's entries, queued by menu number.
    pub action: Option<Interaction>,
}

/// A menu's entries, the sounds its check trees asked for, and why the
/// entries are not the original's when they are not.
#[derive(Debug, Clone)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// The check trees' requests to the sound system, in order.
    pub sounds: Vec<SoundEvent>,
    /// Why the simulation could not list the menu, when the entries are
    /// every entry of the table instead (see `every_entry`).
    pub unchecked: Option<String>,
}

/// What the pie menu offers `person` on `object`: the simulation's
/// listing on the lot (`Lot::pie_menu`, which keeps the changes its check
/// trees make; see the module documentation); or, where the simulation
/// has not ported something the listing needs, every entry of the table,
/// with the reason, and the lot unchanged. Any other failure of the
/// listing is an error.
pub fn entries(
    lot: &mut Lot,
    assets: &Assets,
    person: u16,
    object: u16,
) -> Result<Listing, String> {
    match lot.pie_menu(person, object) {
        Ok(menu) => Ok(Listing {
            entries: named(lot, assets, menu.entries)?,
            sounds: menu.sounds,
            unchecked: None,
        }),
        Err(OrderError::Refused(fault)) if unported(&fault.trap) => Ok(Listing {
            entries: every_entry(lot, assets, object)?,
            sounds: Vec::new(),
            unchecked: Some(fault.to_string()),
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// Whether `trap` is something the simulation has not ported, rather
/// than a failure of the script or a wrong caller. A check tree's fault
/// keeps its trap ([`Trap::CheckTree`]), and is what that trap is: a
/// check that takes its error branch or runs out of instructions is not
/// something unported.
fn unported(trap: &Trap) -> bool {
    trap.unported()
}

/// The entries the simulation's pie menu offered, named: by the name a
/// check tree gave the entry (Add/Change Action String, 0x458789), as it
/// is, or else by the TTAs strings with "*" before a debug entry, as the
/// original's getter does (0x41f167).
fn named(lot: &Lot, assets: &Assets, offered: Vec<MenuEntry>) -> Result<Vec<Entry>, String> {
    let mut labels: BTreeMap<u16, Option<StringTable>> = BTreeMap::new();
    let mut out = Vec::new();
    for MenuEntry { action, name } in offered {
        let menu = u32::try_from(action.table_index)
            .map_err(|_| format!("an offered entry has menu number {}", action.table_index))?;
        let target = u16::try_from(action.target)
            .ok()
            .filter(|&t| lot.world().objects.contains_key(&t))
            .ok_or_else(|| {
                format!(
                    "an offered entry's target {} is not an object",
                    action.target
                )
            })?;
        let guid = lot.world().objects[&target].guid;
        let found = table(lot, guid).ok().flatten();
        let debug = found.as_ref().is_some_and(|f| {
            f.table
                .interactions
                .iter()
                .find(|i| i.string_index == menu)
                .is_some_and(|i| i.flags & DEBUG != 0)
        });
        // The name a check tree gave is shown as it is; an entry without
        // one, or with an empty one, is named by the TTAs strings, with
        // "*" before a debug entry (0x41f167). The offer to join already
        // carries its entry's stored name (0x41efc1), unless a check tree
        // named it, even with an empty name (0x4587d0).
        let named_by_check = name.is_some();
        let label = match name.filter(|n| !n.is_empty()) {
            Some(name) => name,
            None => {
                let strings = labels
                    .entry(target)
                    .or_insert_with(|| found.and_then(|f| self::labels(assets, f.resource, f.id)))
                    .as_ref();
                let label = strings.and_then(|l| l.get(menu as usize, LANGUAGE));
                let stored = action.join == JOINS && !named_by_check && strings.is_some();
                entry_label(label, menu, debug, stored)
            }
        };
        out.push(Entry {
            menu,
            label,
            action: Some(action),
        });
    }
    Ok(out)
}

/// The name of an offered entry without one of its own: its TTAs
/// `label`, or else (a blank one too) its menu number, with "*" before
/// it for a `debug` entry (0x41f167), unless the action already carries
/// the entry's `stored` name, as the offer to join does (0x41efc1): the
/// getter adds the "*" only to an empty name (0x41f174).
///
/// The offer carries one whenever the object's TTAs strings loaded: the
/// original stores the entry's label when its first byte is not 0
/// (0x475bf6), and otherwise a name made from its tree (0x475c77). That
/// made name is shown here as the menu number.
fn entry_label(label: Option<&str>, menu: u32, debug: bool, stored: bool) -> String {
    let name = label
        .filter(|l| !l.trim().is_empty())
        .map_or_else(|| format!("Interaction {menu}"), str::to_owned);
    if debug && !stored {
        format!("*{name}")
    } else {
        name
    }
}

/// The fallback when the simulation cannot list the menu: every entry of
/// the interaction table that orders on `object` are queued from, first
/// to last, each menu number once, except those with flag 0x80 (every
/// Debug and Force entry of the Goths' house has it, and their checks,
/// which are not run here, would refuse them). `lot` gives the table,
/// `assets` the file its labels are in; an entry with no label is named
/// by its number.
fn every_entry(lot: &Lot, assets: &Assets, object: u16) -> Result<Vec<Entry>, String> {
    let target = target(lot, object)?;
    let guid = lot.world().objects[&target].guid;
    let definition = lot
        .catalog()
        .object(guid)
        .ok_or_else(|| format!("no definition for type {guid:#010x}"))?;
    let Some(queued) = lot
        .program()
        .interactions(definition.resource, definition.chunk)?
    else {
        return Ok(Vec::new());
    };
    // The labels are read only where the table found here is the very
    // one the simulation queues from.
    let labels = table(lot, guid)
        .ok()
        .flatten()
        .filter(|found| std::ptr::eq(found.table, queued))
        .and_then(|found| labels(assets, found.resource, found.id));
    let mut out: Vec<Entry> = Vec::new();
    let mut hidden = Vec::new();
    for interaction in &queued.interactions {
        let menu = interaction.string_index;
        // The simulation queues the first entry with a menu number, so
        // that entry's flags are the ones that count.
        if interaction.flags & DEBUG != 0 && !out.iter().any(|e| e.menu == menu) {
            hidden.push(menu);
        }
        if hidden.contains(&menu) || out.iter().any(|e| e.menu == menu) {
            continue;
        }
        let label = labels
            .as_ref()
            .and_then(|l| l.get(menu as usize, LANGUAGE))
            .filter(|l| !l.trim().is_empty())
            .map_or_else(|| format!("Interaction {menu}"), str::to_owned);
        out.push(Entry {
            menu,
            label,
            action: None,
        });
    }
    Ok(out)
}

/// The definition words of the type `guid`.
fn words(lot: &Lot, guid: u32) -> Result<&[i16; DEFINITION_WORDS], String> {
    let definition = lot
        .catalog()
        .object(guid)
        .ok_or_else(|| format!("no definition for type {guid:#010x}"))?;
    lot.program()
        .definition(definition.resource, definition.chunk)
        .ok_or_else(|| format!("no definition words for type {guid:#010x}"))
}

/// The tiles of `id`'s group, lead tile first, or `id` alone if it has
/// one tile (the simulation's `parts`, placement.rs).
fn parts(world: &World, id: u16) -> Result<Vec<u16>, String> {
    let Some(part) = world.objects.get(&id).and_then(|o| o.multipart.as_ref()) else {
        return Ok(vec![id]);
    };
    let lead = if part.parent == 0 {
        id
    } else {
        part.parent as u16
    };
    let mut parts = vec![lead];
    while let Some(&next) = parts.last().and_then(|last| world.next_parts.get(last)) {
        if parts.contains(&next) {
            return Err("the tiles of a group form a loop".into());
        }
        parts.push(next);
    }
    if !parts.contains(&id) || parts.iter().any(|p| !world.objects.contains_key(p)) {
        return Err(format!("object {id} is not among the tiles of its group"));
    }
    Ok(parts)
}

/// The object a Sim's interactions with `id` go to, as the simulation
/// picks it when it queues an order (`interaction_target`, factory.rs,
/// 0x435286): `id` itself, unless it is a tile of a multi-tile object
/// whose definition's word 8 is positive; then the first tile of its
/// group whose word 8 is that number negated, if there is one.
pub fn target(lot: &Lot, id: u16) -> Result<u16, String> {
    let world = lot.world();
    let object = world
        .objects
        .get(&id)
        .ok_or_else(|| format!("no object {id}"))?;
    let own = words(lot, object.guid)?;
    if own[10] != 0 && own[8] > 0 && object.multipart.is_some() {
        for part in parts(world, id)? {
            if words(lot, world.objects[&part].guid)?[8] == -own[8] {
                return Ok(part);
            }
        }
    }
    Ok(id)
}

/// An interaction table and where it is: the ID of its TTAB chunk and the
/// resource whose file holds it.
struct Table<'a> {
    resource: usize,
    id: u16,
    table: &'a InteractionTable,
}

/// The interaction table of the type `guid`, as
/// [`sims_vm::Program::interactions`] finds it: the TTAB its word 7 names
/// (or, for a tile whose word 7 is -1, its group's catalog definition's),
/// in its own file or else its semi-global file. None when the table
/// found here is not the one `Program::interactions` returns.
fn table(lot: &Lot, guid: u32) -> Result<Option<Table<'_>>, String> {
    let program = lot.program();
    let definition = lot
        .catalog()
        .object(guid)
        .ok_or_else(|| format!("no definition for type {guid:#010x}"))?;
    let resource = definition.resource;
    let library = program
        .libraries
        .get(&resource)
        .ok_or_else(|| format!("resource {resource} is not linked"))?;
    let words = library
        .definitions
        .get(&definition.chunk)
        .ok_or_else(|| format!("resource {resource} has no OBJD #{}", definition.chunk))?;
    let mut id = words[7];
    if id == -1 {
        id = library
            .definition_order
            .iter()
            .filter_map(|chunk| library.definitions.get(chunk))
            .find(|parent| words[10] != 0 && parent[10] == words[10] && parent[11] == -1)
            .map_or(0, |parent| parent[7]);
    }
    let Ok(id) = u16::try_from(id) else {
        return Ok(None);
    };
    if id == 0 {
        return Ok(None);
    }
    let semi = program.semi_globals.get(&resource).copied().flatten();
    for resource in std::iter::once(resource).chain(semi) {
        if let Some(found) = program
            .libraries
            .get(&resource)
            .and_then(|l| l.interactions.get(&id))
        {
            let Some(table) = found else {
                return Err(format!("resource {resource}'s TTAB #{id} is not read"));
            };
            // Only the very table the simulation reads counts.
            let simulation = program.interactions(definition.resource, definition.chunk)?;
            return Ok(simulation
                .is_some_and(|t| std::ptr::eq(t, table))
                .then_some(Table {
                    resource,
                    id,
                    table,
                }));
        }
    }
    Ok(None)
}

/// The TTAs strings with the ID `id` in the file of `resource`.
fn labels(assets: &Assets, resource: usize, id: u16) -> Option<StringTable> {
    let file = assets.index().resources.get(resource)?;
    let bytes = file.read().ok()?;
    let iff = Iff::parse(bytes).ok()?;
    let strings = iff.get(b"TTAs", id)?;
    StringTable::parse(iff.data(strings)).ok()
}

/// An open menu: who it is for, what it is on, and the pie it is shown
/// as (`sims_ui::pie`, the original's menu window and object picker).
#[derive(Debug, Clone)]
pub struct Menu {
    pub person: u16,
    pub object: u16,
    pub entries: Vec<Entry>,
    pie: Pie,
    /// The pie's menus as last laid out, the first menu first.
    layouts: Vec<pie::Layout>,
    /// The screen they were laid out on.
    size: [i32; 2],
    /// Where the pointer is to be moved, as the original moves it, until
    /// the host takes it.
    pointer: Option<[i32; 2]>,
    /// Whether the pie has settled into click-to-choose mode (`+0x14e`,
    /// 0x5ebab2, 0x5eb433): false right after opening from a left press
    /// until the opening button is released, then true (and true in a
    /// submenu opened by a release).
    sticky: bool,
    /// Whether the menu had to move to stay on screen (`+0x154`,
    /// 0x5ebab2), cleared on the first release (0x5eb4b3). While set and
    /// `!sticky`, releasing settles the menu instead of choosing
    /// (0x5eb45d).
    moved: bool,
    /// Whether the left mouse button is held down over the pie, which
    /// draws the highlighted label in cyan (0x5eb04c).
    held: bool,
    /// The head of the Sim the menu is for, drawn in each menu's middle
    /// (`People::head`), if it was rendered.
    pub head: Option<Image>,
}

/// What a left release on an open [`Menu`] did (0x5eb433).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// The opening press was released without choosing (within the dead
    /// zone, or after the menu moved to stay on screen); the menu stays
    /// open in click-to-choose mode.
    Settled,
    /// The player chose this entry (its index in [`Menu::entries`]); the
    /// menu is closed.
    Chose(usize),
    /// A submenu opened, or the release went back from one; the menu
    /// stays open on the new level.
    Stays,
    /// The release was elsewhere: the menu is closed.
    Closed,
}

/// Distance from `layout`'s middle to `point`, as a 32-bit float
/// (0x5eb7c9, `+0x164`).
fn distance(layout: &pie::Layout, point: [i32; 2]) -> f32 {
    let dx = f64::from(point[0]) - f64::from(layout.center[0]);
    let dy = f64::from(point[1]) - f64::from(layout.center[1]);
    (dx * dx + dy * dy).sqrt() as f32
}

impl Menu {
    /// The menu of `entries` opened at `at`, the pointer where the player
    /// pressed, and laid out on a screen of `size` with `art` (which
    /// holds [`pie::RESOURCES`] and the label font; nothing is laid out
    /// without the font). An entry whose name has a `/` goes into a
    /// submenu, and only the first [`pie::SHOWN`] of a menu are shown, as
    /// the original's picker does. Where the menu had to move to stay on
    /// the screen, the pointer is to move to its middle.
    ///
    /// Opens as from a held left press (`+0x14e = 0`, 0x48d56c): the
    /// matching release settles the menu if it stays within the dead zone
    /// or the menu moved, or chooses at once on a drag past the dead zone
    /// (0x5eb433).
    pub fn new(
        person: u16,
        object: u16,
        entries: Vec<Entry>,
        at: [i32; 2],
        art: &Art,
        size: [i32; 2],
    ) -> Self {
        let names: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        let pie = Pie::new(&names, at);
        let mut menu = Self {
            person,
            object,
            entries,
            pie,
            layouts: Vec::new(),
            size,
            pointer: None,
            sticky: false,
            moved: false,
            held: true,
            head: None,
        };
        menu.layout(art, size);
        menu.moved = menu.layouts.first().is_some_and(|l| l.moved);
        menu.pointer = menu.moved.then(|| menu.layouts[0].center);
        menu
    }

    /// Lays the menus out again on a screen of `size`.
    pub fn layout(&mut self, art: &Art, size: [i32; 2]) {
        self.size = size;
        self.layouts = self.pie.layout(size, art).unwrap_or_default();
    }

    /// Which menu is open and the entry of it under `point`, to tell
    /// when the highlight changes.
    pub fn hovered(&self, point: [i32; 2]) -> (usize, Option<usize>) {
        (self.layouts.len(), Pie::hit(&self.layouts, point))
    }

    /// The entry (its index in [`Menu::entries`]) highlighted under
    /// `point`: the one a release there would choose.
    pub fn highlighted(&self, point: [i32; 2]) -> Option<usize> {
        self.pie.choice_at(&self.layouts, point)
    }

    /// Whether the left mouse button is currently held over the pie
    /// (drawing the highlighted label in cyan, 0x5eb04c).
    pub fn held(&self) -> bool {
        self.held
    }

    /// A left press on the open pie (0x5eb3d1, with `+0x170` clear as
    /// 0x518665 sets it): does not choose or close, only marks the button
    /// held so the highlighted entry draws in cyan. True if that state
    /// changed.
    pub fn press(&mut self) -> bool {
        let changed = !self.held;
        self.held = true;
        changed
    }

    /// Whether a left release at `point` will choose, open/leave a
    /// submenu, or close (`0x5eb433`: already settled, or dragged past
    /// the dead zone without the menu having moved on screen), rather
    /// than settling the opening press.
    pub fn will_choose(&self, point: [i32; 2]) -> bool {
        if self.sticky {
            return true;
        }
        !self.moved
            && self
                .layouts
                .last()
                .is_some_and(|top| distance(top, point) > pie::DEAD_ZONE as f32)
    }

    /// A left release at `point` (0x5eb433), as the picker takes it.
    ///
    /// While the menu is still on its opening press (`!sticky`), a
    /// release within the dead zone or after the menu moved to stay on
    /// screen settles it into click-to-choose mode (`Release::Settled`);
    /// a release dragged past the dead zone on an unmoved menu acts at
    /// once. A submenu opened or closed is laid out at once and stays
    /// settled (`sticky`), with the pointer moved to a newly opened
    /// submenu's middle (0x48d9c0).
    pub fn release(&mut self, art: &Art, point: [i32; 2]) -> Release {
        self.held = false;
        let choose = self.will_choose(point);
        self.moved = false;
        if !choose {
            self.sticky = true;
            return Release::Settled;
        }
        match self.pie.press(&self.layouts, point) {
            Pressed::Chose(choice) => Release::Chose(choice),
            pressed @ (Pressed::Opened { .. } | Pressed::Back) => {
                self.sticky = true;
                self.layout(art, self.size);
                if matches!(pressed, Pressed::Opened { .. }) {
                    self.pointer = self.layouts.last().map(|l| l.center);
                }
                Release::Stays
            }
            Pressed::Closed => Release::Closed,
        }
    }

    /// Where the pointer is to be moved, once.
    pub fn take_pointer(&mut self) -> Option<[i32; 2]> {
        self.pointer.take()
    }

    /// The labels of the menu on top as last laid out, with their boxes:
    /// the ones a press can choose.
    pub fn labels(&self) -> Vec<(&str, Rect)> {
        self.layouts
            .last()
            .map(|top| {
                top.labels
                    .iter()
                    .filter(|l| l.enabled)
                    .map(|l| (l.text.as_str(), l.rect))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The size of the face in each menu's middle, which the head is
    /// drawn to fill: the face bitmap's.
    pub fn face_size(&self) -> [u32; 2] {
        let face = self
            .layouts
            .first()
            .map_or(pie::FACE_SIZE, |l| [l.face.width, l.face.height]);
        face.map(|n| u32::try_from(n).unwrap_or(0))
    }

    /// Draws the menus as last laid out, with [`Menu::head`] over the face
    /// bitmap in the middle.
    pub fn draw(&self, frame: &mut Frame, art: &Art, pointer: Option<[i32; 2]>) {
        Pie::draw(
            frame,
            art,
            &self.layouts,
            self.head.as_ref(),
            pointer,
            self.held,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::Location;

    fn entry(menu: u32) -> Entry {
        Entry {
            menu,
            label: format!("entry {menu}"),
            action: None,
        }
    }

    #[test]
    fn only_what_is_not_ported_falls_back_to_every_entry() {
        // A check tree's fault, as the simulation reports it: the fault
        // inside keeps its trap.
        let check = |trap: Trap, at: Option<Location>| Trap::CheckTree {
            routine: None,
            tree: 0x1012,
            object: 118,
            fault: Box::new(sims_vm::Fault {
                object: 21,
                at,
                trap,
            }),
        };
        let at = Some(Location {
            tree: 0x1012,
            node: 5,
        });
        let not_ported = [
            Trap::Unsupported("a pet's pie menu".into()),
            Trap::Unsupported("check tree 0x2000, which names a primitive".into()),
            Trap::UnsupportedScope { scope: 12 },
            Trap::Unimplemented { opcode: 0x50 },
        ];
        let failures = [
            Trap::ErrorBranch,
            Trap::BudgetExhausted { budget: 1 },
            Trap::Invalid("object 5 is not a person".into()),
            Trap::MissingTree { id: 0x1012 },
            Trap::StackOverflow,
        ];
        for trap in not_ported.clone() {
            assert!(unported(&trap), "{trap:?}");
            assert!(unported(&check(trap.clone(), at)), "{trap:?}");
            assert!(unported(&check(trap.clone(), None)), "{trap:?}");
            assert!(unported(&check(check(trap.clone(), at), at)), "{trap:?}");
        }
        for trap in failures {
            assert!(!unported(&trap), "{trap:?}");
            assert!(!unported(&check(trap.clone(), at)), "{trap:?}");
            assert!(!unported(&check(trap.clone(), None)), "{trap:?}");
            assert!(!unported(&check(check(trap.clone(), at), at)), "{trap:?}");
        }
    }

    /// Art with only the pie's label font: every glyph 4 wide, lines 23
    /// high, as `variablesans_12`'s are.
    fn art() -> Art {
        let glyphs = (32u16..127)
            .map(|code| sims_formats::ffn::Glyph {
                code,
                width: 4,
                height: 23,
                atlas_x: 0,
                atlas_y: 0,
                first_bearing: 0,
                bearing_x: 0,
                bearing_y: 0,
            })
            .collect();
        let mut art = Art::default();
        art.insert_font(
            pie::FONT,
            sims_ui::font::Font::new(sims_formats::ffn::Font {
                flags: 9,
                glyphs,
                width: 4,
                height: 23,
                coverage: vec![15; 4 * 23],
            }),
        );
        art
    }

    fn named(labels: &[&str]) -> Vec<Entry> {
        labels
            .iter()
            .enumerate()
            .map(|(n, l)| Entry {
                label: (*l).into(),
                ..entry(u32::try_from(n).unwrap())
            })
            .collect()
    }

    fn centre(r: Rect) -> [i32; 2] {
        [r.x + r.width / 2, r.y + r.height / 2]
    }

    const STUDIES: [&str; 3] = ["Read a Book", "Study/Cooking", "Study/Mechanical"];

    #[test]
    fn a_release_chooses_an_entry_by_its_place_in_the_listing() {
        let art = art();
        let mut menu = Menu::new(1, 2, named(&STUDIES), [400, 300], &art, [800, 600]);
        // Laid out where it opened: the pointer stays, button held until
        // the opening press releases.
        assert_eq!(menu.take_pointer(), None);
        assert!(menu.held());
        // Releasing at the center settles the menu without closing it.
        assert_eq!(menu.release(&art, [400, 300]), Release::Settled);
        assert!(!menu.held());
        // Two entries in the first menu: the book, and the studies'
        // submenu.
        assert_eq!(menu.layouts.len(), 1);
        assert_eq!(menu.layouts[0].labels.len(), 2);
        let study = centre(menu.layouts[0].labels[1].rect);
        assert_eq!(menu.hovered(study), (1, Some(1)));
        // A press marks held without opening the submenu; release opens it.
        assert!(menu.press());
        assert!(menu.held());
        assert_eq!(menu.layouts.len(), 1);
        assert_eq!(menu.release(&art, study), Release::Stays);
        assert!(!menu.held());
        // The submenu is laid out at once, and the pointer goes to its
        // middle.
        assert_eq!(menu.layouts.len(), 2);
        assert_eq!(menu.take_pointer(), Some(menu.layouts[1].center));
        assert_eq!(menu.take_pointer(), None);
        // Mechanical, the second entry of the submenu, is the third entry
        // listed. Pressing then releasing chooses it.
        let mechanical = centre(menu.layouts[1].labels[1].rect);
        assert_eq!(menu.hovered(mechanical), (2, Some(1)));
        assert!(menu.press());
        assert_eq!(menu.release(&art, mechanical), Release::Chose(2));

        // Pressing on an entry then dragging off before releasing closes
        // without choosing.
        let mut menu = Menu::new(1, 2, named(&["Read a Book"]), [400, 300], &art, [800, 600]);
        assert_eq!(menu.release(&art, [400, 300]), Release::Settled);
        let book = centre(menu.layouts[0].labels[0].rect);
        assert_eq!(menu.hovered(book), (1, Some(0)));
        assert!(menu.press());
        assert_eq!(menu.release(&art, [5, 5]), Release::Closed);

        // Press-drag-release from an unmoved menu past the dead zone
        // chooses on the opening press's release (0x5eb433).
        let mut menu = Menu::new(1, 2, named(&["Read a Book"]), [400, 300], &art, [800, 600]);
        let book = centre(menu.layouts[0].labels[0].rect);
        assert!(menu.will_choose(book));
        assert_eq!(menu.release(&art, book), Release::Chose(0));

        // Press-drag-release past the dead zone into empty space closes.
        let mut menu = Menu::new(1, 2, named(&["Read a Book"]), [400, 300], &art, [800, 600]);
        assert_eq!(menu.release(&art, [5, 5]), Release::Closed);
    }

    #[test]
    fn a_second_release_finds_the_submenu_the_first_opened() {
        let art = art();
        let mut menu = Menu::new(1, 2, named(&STUDIES), [400, 300], &art, [800, 600]);
        assert_eq!(menu.release(&art, [400, 300]), Release::Settled);
        let study = centre(menu.layouts[0].labels[1].rect);
        assert_eq!(menu.release(&art, study), Release::Stays);
        // Released again where Study was, with no drawing between: that is
        // the submenu's middle, which goes back.
        assert_eq!(menu.release(&art, study), Release::Stays);
        assert_eq!(menu.layouts.len(), 1);
    }

    #[test]
    fn a_menu_moved_to_stay_on_the_screen_moves_the_pointer() {
        let art = art();
        // Opened near the top: the first menu moves down, and so does the
        // pointer. Even if the opening release lands past the dead zone,
        // `moved` settles the menu instead of choosing (0x5eb45d).
        let mut menu = Menu::new(1, 2, named(&STUDIES), [400, 20], &art, [800, 600]);
        let first = menu.layouts[0].clone();
        assert!(first.moved);
        assert_eq!(menu.take_pointer(), Some(first.center));
        let study = centre(first.labels[1].rect);
        assert!(!menu.will_choose(study));
        assert_eq!(menu.release(&art, study), Release::Settled);
        assert_eq!(menu.layouts.len(), 1);
        // Once settled, releasing on Study opens the submenu.
        assert!(menu.will_choose(study));
        assert_eq!(menu.release(&art, study), Release::Stays);
        assert_eq!(menu.layouts.len(), 2);

        // A submenu opened from the top entry, near the top, moves down;
        // the pointer goes to where it moved, not to the entry.
        let names = ["Study/Cooking", "Study/Mechanical", "Read a Book"];
        let mut menu = Menu::new(1, 2, named(&names), [400, 130], &art, [800, 600]);
        assert_eq!(menu.take_pointer(), None);
        assert_eq!(menu.release(&art, [400, 130]), Release::Settled);
        let study = centre(menu.layouts[0].labels[0].rect);
        assert_eq!(menu.release(&art, study), Release::Stays);
        let submenu = menu.layouts[1].clone();
        assert!(submenu.moved, "{submenu:?}");
        assert_ne!(submenu.center, study);
        assert_eq!(menu.take_pointer(), Some(submenu.center));
        // There a release goes back, as it does in the original (a submenu
        // opened by release starts settled).
        assert_eq!(menu.release(&art, submenu.center), Release::Stays);
        assert_eq!(menu.layouts.len(), 1);
    }

    #[test]
    fn a_debug_entry_is_starred_unless_it_carries_its_stored_name() {
        assert_eq!(entry_label(Some("Eat"), 3, false, false), "Eat");
        assert_eq!(entry_label(Some("Eat"), 3, true, false), "*Eat");
        assert_eq!(entry_label(Some("Eat"), 3, true, true), "Eat");
        assert_eq!(entry_label(Some(" "), 3, true, true), "Interaction 3");
        assert_eq!(entry_label(Some(" "), 3, true, false), "*Interaction 3");
        assert_eq!(entry_label(None, 3, false, true), "Interaction 3");
    }
}
