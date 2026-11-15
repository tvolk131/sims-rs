//! The pie menu: the interactions of an object the player clicked, for
//! the selected Sim to do.
//!
//! [`entries`] lists what the original's pie menu offers
//! ([`sims_vm::Interpreter::pie_menu`], 0x48d392): each entry of the
//! object's interaction table whose check passes for that Sim, in order.
//! Each is named by the TTAs strings with the ID of the TTAB it comes
//! from, by its menu number, with "*" before it when the entry has flag
//! 0x80, as the original names it. An entry with no string is named by
//! its number; a person's TTAs names only its first entry (Talk), so a
//! person's menu is mostly numbers, as where the original reads the rest
//! is not found yet.
//!
//! **Listed on a copy.** Listing the menu runs the entries' check trees,
//! which change the lot and may draw random numbers. The project's rule
//! (`docs/vision.md`, "Other rules") runs them against a throwaway copy
//! of the state, never the shared simulation, so the game lists the menu
//! on a copy of the lot's world when the object is clicked and drops the
//! copy's changes.
//!
//! **Still a stopgap in two ways.** Choosing an entry queues it by its
//! menu number (`Lot::queue_interaction`), which builds the action again
//! from the lot's own world without its check: where an earlier check on
//! the copy changed what an action is built from (whether the person is
//! a visitor, which sets its attenuation), the queued action differs from
//! the one offered. `Lot` has no way yet to queue the offered action
//! itself (`Interpreter::pick`). And the menu is a plain list at the
//! pointer, in columns when it is taller than the view, not the
//! original's pie.
//!
//! Where the simulation cannot list a menu yet (it fails loudly on what
//! it has not ported: a check tree adding an entry, as the fridge's does;
//! an object that offers nothing; a version 0 table; a pet; joining; an
//! unported scope or primitive), the menu falls back to every entry of
//! the table without checks (`every_entry`), and [`Listing::unchecked`]
//! says why. Any other failure (a person who is not one, a check tree
//! taking its error branch or running out of instructions) is an error:
//! no menu is shown.
//!
//! Which TTAB an object's entries come from, and which tile of a
//! multi-tile object they go to, the simulation decides in code it keeps
//! private ([`sims_vm::Program::interactions`], `interaction_target`);
//! `table` and [`target`] repeat that from the program's public data, for
//! the labels and the fallback.

use sims_formats::behavior::InteractionTable;
use sims_formats::iff::Iff;
use sims_formats::saved_objects::Interaction;
use sims_formats::strings::StringTable;
use sims_render::Assets;
use sims_sim::World;
use sims_ui::font::Font;
use sims_ui::frame::{Frame, Rect};
use sims_vm::DEFINITION_WORDS;
use sims_vm::boot::Lot;
use sims_vm::{Interpreter, Stream, Trap};
use std::collections::BTreeMap;

/// The language the labels are read in (US English).
const LANGUAGE: u8 = 1;

/// The entry flag that puts "*" before the entry's name.
const DEBUG: u32 = 0x80;

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The menu number the simulation queues by (the entry's string
    /// index).
    pub menu: u32,
    pub label: String,
}

/// A menu's entries, and why they are not the original's when they are
/// not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// Why the simulation could not list the menu, when the entries are
    /// every entry of the table instead (see `every_entry`).
    pub unchecked: Option<String>,
}

/// What the pie menu offers `person` on `object`: the simulation's
/// listing (see the module documentation), listed on a copy of the lot's
/// world so the lot is not changed; or, where the simulation has not
/// ported something the listing needs, every entry of the table, with
/// the reason. Any other failure of the listing is an error.
pub fn entries(lot: &Lot, assets: &Assets, person: u16, object: u16) -> Result<Listing, String> {
    let interpreter = Interpreter::new(lot.program(), lot.catalog(), Stream::Global);
    match interpreter.pie_menu(&mut lot.world().clone(), person, object) {
        Ok(offered) => Ok(Listing {
            entries: named(lot, assets, offered)?,
            unchecked: None,
        }),
        Err(fault) if unported(&fault.trap) => Ok(Listing {
            entries: every_entry(lot, assets, object)?,
            unchecked: Some(fault.to_string()),
        }),
        Err(fault) => Err(fault.to_string()),
    }
}

/// Whether `trap` is something the simulation has not ported, rather
/// than a failure of the script or a wrong caller.
fn unported(trap: &Trap) -> bool {
    matches!(
        trap,
        Trap::Unsupported(_) | Trap::UnsupportedScope { .. } | Trap::Unimplemented { .. }
    )
}

/// The entries the simulation's pie menu offered, named.
fn named(lot: &Lot, assets: &Assets, offered: Vec<Interaction>) -> Result<Vec<Entry>, String> {
    let mut labels: BTreeMap<u16, Option<StringTable>> = BTreeMap::new();
    let mut out = Vec::new();
    for action in offered {
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
        let strings = labels
            .entry(target)
            .or_insert_with(|| found.and_then(|f| self::labels(assets, f.resource, f.id)));
        let name = strings
            .as_ref()
            .and_then(|l| l.get(menu as usize, LANGUAGE))
            .filter(|l| !l.trim().is_empty())
            .map_or_else(|| format!("Interaction {menu}"), str::to_owned);
        let label = if debug { format!("*{name}") } else { name };
        out.push(Entry { menu, label });
    }
    Ok(out)
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
        out.push(Entry { menu, label });
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

/// The menu's text size and colors.
pub const SIZE: u8 = 10;
const ROW: i32 = SIZE as i32 + 9;
const PAD: i32 = 8;
const BACK: [u8; 3] = [24, 40, 88];
const HOVER: [u8; 3] = [64, 96, 176];
const TEXT: [u8; 3] = [255, 255, 255];
const FAINT: [u8; 3] = [160, 170, 200];
const EDGE: [u8; 3] = [200, 210, 240];

/// The rows `count` entries `width` pixels wide take, opened at `at` on a
/// canvas of `size`: columns of as many rows as fit, left to right, as
/// many columns as fit, moved to stay on the canvas where they can. Fewer
/// rows than `count` when they do not all fit.
fn place(count: usize, width: i32, at: [i32; 2], size: [i32; 2]) -> Vec<Rect> {
    let count = i32::try_from(count).unwrap_or(i32::MAX);
    let per_column = (size[1] / ROW).clamp(1, count.max(1));
    let fit = (size[0] / width.max(1)).max(1);
    let columns = ((count + per_column - 1) / per_column).min(fit);
    let count = count.min(per_column.saturating_mul(columns));
    let x = at[0].min(size[0] - width * columns).max(0);
    let y = at[1].min(size[1] - ROW * per_column).max(0);
    (0..count)
        .map(|i| {
            let (column, row) = (i / per_column, i % per_column);
            Rect::new(x + width * column, y + ROW * row, width, ROW)
        })
        .collect()
}

/// An open menu: who it is for, what it is on, and where it is drawn.
#[derive(Debug, Clone)]
pub struct Menu {
    pub person: u16,
    pub object: u16,
    pub entries: Vec<Entry>,
    /// Where it was opened (canvas pixels).
    pub at: [i32; 2],
    /// The rows of the entries shown, first to last, once laid out.
    rows: Vec<Rect>,
    /// When not every entry fits: the last row, which says how many are
    /// not shown and chooses nothing.
    more: Option<(Rect, usize)>,
}

impl Menu {
    pub fn new(person: u16, object: u16, entries: Vec<Entry>, at: [i32; 2]) -> Self {
        Self {
            person,
            object,
            entries,
            at,
            rows: Vec::new(),
            more: None,
        }
    }

    /// The label of the row that stands for the entries not shown.
    fn more_label(hidden: usize) -> String {
        format!("({hidden} more not shown)")
    }

    /// Lays the rows out with `font` on a canvas of `size`: columns of
    /// as many rows as fit, at the pointer, moved to stay on the canvas.
    /// If they do not all fit, the last row that does says how many more
    /// there are.
    pub fn layout(&mut self, font: &Font, size: [i32; 2]) {
        let width = self
            .entries
            .iter()
            .map(|e| font.width(&e.label))
            .chain(std::iter::once(
                font.width(&Self::more_label(self.entries.len())),
            ))
            .max()
            .unwrap_or(0)
            + 2 * PAD;
        self.arrange(width, size);
    }

    fn arrange(&mut self, width: i32, size: [i32; 2]) {
        let mut rows = place(self.entries.len(), width, self.at, size);
        self.more = None;
        if rows.len() < self.entries.len()
            && let Some(last) = rows.pop()
        {
            self.more = Some((last, self.entries.len() - rows.len()));
        }
        self.rows = rows;
    }

    /// The index of the entry whose row is under `point`.
    pub fn row(&self, point: [i32; 2]) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.contains(point[0], point[1]))
    }

    /// Whether the menu covers `point`.
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.row(point).is_some()
            || self
                .more
                .is_some_and(|(r, _)| r.contains(point[0], point[1]))
    }

    pub fn draw(&self, frame: &mut Frame, font: &Font, pointer: Option<[i32; 2]>) {
        let hovered = pointer.and_then(|p| self.row(p));
        let all = || self.rows.iter().chain(self.more.as_ref().map(|(r, _)| r));
        if let Some(first) = self.rows.first().or(self.more.as_ref().map(|(r, _)| r)) {
            let right = all().map(|r| r.x + r.width).max().unwrap_or(0);
            let bottom = all().map(|r| r.y + r.height).max().unwrap_or(0);
            let (x, y) = (first.x - 1, first.y - 1);
            frame.fill(Rect::new(x, y, right - x + 1, bottom - y + 1), EDGE);
        }
        for (i, (row, entry)) in self.rows.iter().zip(&self.entries).enumerate() {
            let back = if hovered == Some(i) { HOVER } else { BACK };
            frame.fill(*row, back);
            font.draw(frame, &entry.label, [row.x + PAD, row.y + 2], TEXT);
        }
        if let Some((row, hidden)) = self.more {
            frame.fill(row, BACK);
            font.draw(
                frame,
                &Self::more_label(hidden),
                [row.x + PAD, row.y + 2],
                FAINT,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(menu: u32) -> Entry {
        Entry {
            menu,
            label: format!("entry {menu}"),
        }
    }

    #[test]
    fn only_what_is_not_ported_falls_back_to_every_entry() {
        assert!(unported(&Trap::Unsupported("a pet's pie menu".into())));
        assert!(unported(&Trap::UnsupportedScope { scope: 12 }));
        assert!(unported(&Trap::Unimplemented { opcode: 0x50 }));
        for trap in [
            Trap::ErrorBranch,
            Trap::BudgetExhausted { budget: 1 },
            Trap::Invalid("object 5 is not a person".into()),
            Trap::MissingTree { id: 0x1012 },
            Trap::StackOverflow,
        ] {
            assert!(!unported(&trap), "{trap:?}");
        }
    }

    #[test]
    fn a_tall_menu_wraps_into_columns_on_the_canvas() {
        // Four rows fit; ten entries take three columns, moved left and up
        // to stay on the canvas.
        let rows = place(10, 40, [500, 500], [200, 4 * ROW + 3]);
        assert_eq!(rows.len(), 10);
        assert_eq!((rows[0].x, rows[0].y), (80, 3));
        assert_eq!((rows[3].x, rows[3].y), (80, 3 + 3 * ROW));
        assert_eq!((rows[4].x, rows[4].y), (120, 3));
        assert_eq!((rows[9].x, rows[9].y), (160, 3 + ROW));
        // A short menu stays where it was opened.
        let rows = place(2, 40, [10, 20], [200, 200]);
        assert_eq!((rows[1].x, rows[1].y), (10, 20 + ROW));
        assert!(place(0, 40, [10, 20], [200, 200]).is_empty());
    }

    #[test]
    fn a_menu_wider_than_the_canvas_shows_what_fits() {
        // Four rows and two columns fit: eight of twenty entries.
        let rows = place(20, 40, [500, 500], [90, 4 * ROW]);
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|r| r.x >= 0 && r.x + r.width <= 90));
        // A column wider than the canvas still shows one column.
        assert_eq!(place(20, 400, [0, 0], [90, 4 * ROW]).len(), 4);

        // The menu gives the last row that fits to how many are not shown,
        // and a press there chooses nothing.
        let mut menu = Menu::new(1, 2, (0..20).map(entry).collect(), [500, 500]);
        menu.arrange(40, [90, 4 * ROW]);
        assert_eq!(menu.rows.len(), 7);
        let (more, hidden) = menu.more.unwrap();
        assert_eq!(hidden, 13);
        let inside = [more.x + 1, more.y + 1];
        assert!(menu.covers(inside));
        assert_eq!(menu.row(inside), None);
        // All of them fit on a big canvas.
        menu.arrange(40, [400, 400]);
        assert_eq!((menu.rows.len(), menu.more), (20, None));
    }

    #[test]
    fn a_press_on_a_row_chooses_its_entry() {
        let mut menu = Menu::new(1, 2, vec![entry(0), entry(3)], [10, 20]);
        menu.rows = vec![Rect::new(10, 20, 50, ROW), Rect::new(10, 20 + ROW, 50, ROW)];
        assert_eq!(menu.row([11, 21]), Some(0));
        assert_eq!(menu.row([11, 21 + ROW]), Some(1));
        assert_eq!(menu.row([5, 21]), None);
        assert!(!menu.covers([11, 20 + 2 * ROW]));
    }
}
