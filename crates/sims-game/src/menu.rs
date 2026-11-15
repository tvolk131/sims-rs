//! The pie menu: the interactions of an object the player clicked, for
//! the selected Sim to do.
//!
//! **A stopgap.** The original offers only the entries whose check passes
//! for that Sim (`sims_vm::orders`, step 3), and draws them as a pie. Until
//! the simulation says which entries it would offer, [`entries`] lists
//! every entry of the interaction table the simulation queues from, each
//! menu number once, named by the TTAs strings with that table's ID,
//! except those with flag 0x80. Every Debug and Force entry of the Goths'
//! house has that flag, and no entry a player is offered there does, so it
//! is taken to mark the entries only the debug menu shows
//! (**unverified**). The menu is a plain list at the pointer, in columns
//! when it is taller than the view. An entry the original would not offer
//! may be listed, and choosing it queues it anyway. A person's TTAs names
//! only its first entry (Talk), so a person's menu is mostly numbers;
//! where the original reads the rest is not found yet.
//!
//! Which object's table that is, and which table, the simulation decides
//! in code it keeps private (`interaction_target`, 0x435286, and
//! [`sims_vm::Program::interactions`]); [`target`] and `table` repeat
//! that from the program's public data until it offers them.

use sims_formats::behavior::InteractionTable;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_render::Assets;
use sims_sim::World;
use sims_ui::font::Font;
use sims_ui::frame::{Frame, Rect};
use sims_vm::DEFINITION_WORDS;
use sims_vm::boot::Lot;

/// The language the labels are read in (US English).
const LANGUAGE: u8 = 1;

/// The entry flag taken to mark debug-only entries (see the module
/// documentation).
const DEBUG: u32 = 0x80;

/// The definition word that groups the tiles of a multi-tile object.
const GROUP: usize = 10;

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The menu number the simulation queues by (the entry's string
    /// index).
    pub menu: u32,
    pub label: String,
}

/// The entries of the interaction table that orders on `object` are
/// queued from, first to last, each menu number once. `lot` gives the
/// table, `assets` the file its labels are in; an entry with no label is
/// named by its number.
pub fn entries(lot: &Lot, assets: &Assets, object: u16) -> Result<Vec<Entry>, String> {
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
    if own[GROUP] != 0 && own[8] > 0 && object.multipart.is_some() {
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
/// in its own file or else its semi-global file.
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
            .find(|parent| words[GROUP] != 0 && parent[GROUP] == words[GROUP] && parent[11] == -1)
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
            return match found {
                Some(table) => Ok(Some(Table {
                    resource,
                    id,
                    table,
                })),
                None => Err(format!("resource {resource}'s TTAB #{id} is not read")),
            };
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
