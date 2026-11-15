//! The pie menu: the interactions of an object the player clicked, for
//! the selected Sim to do.
//!
//! **A stopgap.** The original offers only the entries whose check passes
//! for that Sim (`sims_vm::orders`, step 3), and draws them as a pie. Until
//! the simulation says which entries it would offer, [`entries`] lists
//! every entry of the object's interaction table (the table the
//! simulation queues from, [`sims_vm::Program::interactions`]), each menu
//! number once, named by the object file's TTAs strings, except those
//! with flag 0x80. Every Debug and Force entry of the Goths' house has
//! that flag, and no entry a player is offered there does, so it is
//! taken to mark the entries only the debug menu shows (**unverified**).
//! The menu is a plain list at the pointer, in columns when it is taller
//! than the view. An entry the original would not offer may be listed,
//! and choosing it queues it anyway.

use sims_formats::behavior::InteractionTable;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_render::Assets;
use sims_ui::font::Font;
use sims_ui::frame::{Frame, Rect};
use sims_vm::boot::Lot;

/// The language the labels are read in (US English).
const LANGUAGE: u8 = 1;

/// The entry flag taken to mark debug-only entries (see the module
/// documentation).
const DEBUG: u32 = 0x80;

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The menu number the simulation queues by (the entry's string
    /// index).
    pub menu: u32,
    pub label: String,
}

/// The entries of `object`'s interaction table, first to last, each menu
/// number once. `lot` gives the table, `assets` the object's file for the
/// labels; an entry with no label is named by its number.
pub fn entries(lot: &Lot, assets: &Assets, object: u16) -> Result<Vec<Entry>, String> {
    let guid = lot
        .world()
        .objects
        .get(&object)
        .ok_or_else(|| format!("no object {object}"))?
        .guid;
    let definition = lot
        .catalog()
        .object(guid)
        .ok_or_else(|| format!("no definition for type {guid:#010x}"))?;
    let Some(table) = lot
        .program()
        .interactions(definition.resource, definition.chunk)?
    else {
        return Ok(Vec::new());
    };
    let labels = labels(assets, guid, table);
    let mut out: Vec<Entry> = Vec::new();
    let mut hidden = Vec::new();
    for interaction in &table.interactions {
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

/// The TTAs strings that go with `table`: those with the ID of the TTAB
/// that reads as `table`, in the object's own file or else its
/// semi-global file.
fn labels(assets: &Assets, guid: u32, table: &InteractionTable) -> Option<StringTable> {
    let catalog = assets.catalog();
    let own = catalog.object(guid)?.resource;
    let semi = catalog.semi_globals.get(&own).copied().flatten();
    let menus = |t: &InteractionTable| -> Vec<u32> {
        t.interactions.iter().map(|i| i.string_index).collect()
    };
    let wanted = menus(table);
    for resource in std::iter::once(own).chain(semi) {
        let Some(file) = assets.index().resources.get(resource) else {
            continue;
        };
        let Some(iff) = file.read().ok().and_then(|b| Iff::parse(b).ok()) else {
            continue;
        };
        let found = iff.all(b"TTAB").find(|chunk| {
            InteractionTable::parse(iff.data(chunk)).is_ok_and(|t| menus(&t) == wanted)
        });
        if let Some(chunk) = found
            && let Some(strings) = iff.get(b"TTAs", chunk.id)
        {
            return StringTable::parse(iff.data(strings)).ok();
        }
    }
    None
}

/// The menu's text size and colors.
pub const SIZE: u8 = 10;
const ROW: i32 = SIZE as i32 + 9;
const PAD: i32 = 8;
const BACK: [u8; 3] = [24, 40, 88];
const HOVER: [u8; 3] = [64, 96, 176];
const TEXT: [u8; 3] = [255, 255, 255];
const EDGE: [u8; 3] = [200, 210, 240];

/// The rows of `count` entries `width` pixels wide, opened at `at` on a
/// canvas of `size`: columns of as many rows as fit, left to right,
/// moved to stay on the canvas where they can.
fn place(count: usize, width: i32, at: [i32; 2], size: [i32; 2]) -> Vec<Rect> {
    let count = i32::try_from(count).unwrap_or(i32::MAX);
    let per_column = (size[1] / ROW).clamp(1, count.max(1));
    let columns = (count + per_column - 1) / per_column;
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
    /// Each entry's row, once laid out.
    rows: Vec<Rect>,
}

impl Menu {
    pub fn new(person: u16, object: u16, entries: Vec<Entry>, at: [i32; 2]) -> Self {
        Self {
            person,
            object,
            entries,
            at,
            rows: Vec::new(),
        }
    }

    /// Lays the rows out with `font` on a canvas of `size`: columns of
    /// as many rows as fit, at the pointer, moved to stay on the canvas.
    pub fn layout(&mut self, font: &Font, size: [i32; 2]) {
        let width = self
            .entries
            .iter()
            .map(|e| font.width(&e.label))
            .max()
            .unwrap_or(0)
            + 2 * PAD;
        self.rows = place(self.entries.len(), width, self.at, size);
    }

    /// The row under `point`.
    fn row(&self, point: [i32; 2]) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.contains(point[0], point[1]))
    }

    /// Whether the menu covers `point`.
    pub fn covers(&self, point: [i32; 2]) -> bool {
        self.row(point).is_some()
    }

    /// The menu number chosen by a press at `point`, if it is on a row.
    pub fn press(&self, point: [i32; 2]) -> Option<u32> {
        self.row(point).map(|i| self.entries[i].menu)
    }

    pub fn draw(&self, frame: &mut Frame, font: &Font, pointer: Option<[i32; 2]>) {
        let hovered = pointer.and_then(|p| self.row(p));
        if let Some(first) = self.rows.first() {
            let right = self.rows.iter().map(|r| r.x + r.width).max().unwrap_or(0);
            let bottom = self.rows.iter().map(|r| r.y + r.height).max().unwrap_or(0);
            let (x, y) = (first.x - 1, first.y - 1);
            frame.fill(Rect::new(x, y, right - x + 1, bottom - y + 1), EDGE);
        }
        for (i, (row, entry)) in self.rows.iter().zip(&self.entries).enumerate() {
            let back = if hovered == Some(i) { HOVER } else { BACK };
            frame.fill(*row, back);
            font.draw(frame, &entry.label, [row.x + PAD, row.y + 2], TEXT);
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
    fn a_press_on_a_row_chooses_its_entry() {
        let mut menu = Menu::new(1, 2, vec![entry(0), entry(3)], [10, 20]);
        menu.rows = vec![Rect::new(10, 20, 50, ROW), Rect::new(10, 20 + ROW, 50, ROW)];
        assert_eq!(menu.press([11, 21]), Some(0));
        assert_eq!(menu.press([11, 21 + ROW]), Some(3));
        assert_eq!(menu.press([5, 21]), None);
        assert!(!menu.covers([11, 20 + 2 * ROW]));
    }
}
