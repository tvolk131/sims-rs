//! The ambient-score player: birds, dogs, wind and traffic outdoors,
//! played from an FSC score (see [`crate::fsc`]).
//!
//! Everything here follows the original's own player (`cFreshPlayer`,
//! vtable 0x671618, cells 0x57978c on), read from a memory image of the
//! running game (never written, not traced at run time), marked *(code)*
//! where it matters. FreeSO's FSC player (public notes) was read for
//! orientation only; its authors call its randomisation "mostly guessed",
//! and nothing here is taken from it.
//!
//! A score is a grid of `rows` by `cols` cells, each a sample with its
//! volume, pan, pitch range and chance. A cursor (row `x`, column `y`)
//! and a window around it (`SelY` rows and `SelX` columns either side)
//! say which cells may play. The game moves the cursor: the column with
//! the hour (see [`HOUR_COLUMNS`]) and the row with a value it computes
//! from the camera (see [`crate::player::Player::set_ambience_view`]).
//!
//! *(code)* How it plays:
//!
//! - Time runs in beats of `60000 / tempo` milliseconds from a timer the
//!   player starts with it. The beat counts 1 to `BPB` and around. The
//!   player is updated once per mixing block here (once per frame in the
//!   original) and plays at most one beat per update, once the beat is
//!   due; a timer past 24 hours starts the count again.
//! - On beat `BPB - 1` (and `BPB - 6`) it chooses cells. The cells that
//!   are not idle, less those marked to go, are counted; while that is
//!   below `max` (scaled by the player's percentage, 100), it picks a cell
//!   and starts it, and when it is also below the target, it may try once
//!   more; then the same for a second cell. The target is `min` (scaled by
//!   the percentage) plus `DifX` and `DifY` shares of the cursor's place,
//!   at most the number of groups and ungrouped cells in the window.
//! - When `pri` is not 0 and as many cells play as the target, one
//!   playing cell (first one outside the window, from a random start,
//!   else a random one inside) is first marked to go, and its group
//!   preferred for the next pick. No score in the install sets `pri`.
//! - A pick scans the window from a random cell, row by row, wrapping, and
//!   weighs each cell: 0 when it is empty, not idle, outside the window,
//!   in a group that has a cell playing that is not marked to go, or
//!   excluded by a playing cell that is not marked to go; otherwise
//!   `prob` (clamped to 0..20) decides: 0 never, 20 always (weight 100,
//!   or 101 in the preferred group), the preferred group 2, and the rest 1
//!   with a chance of `CHANCES[prob] / cells in the window` out of 32,768.
//!   The first cell of the highest weight wins.
//! - A started cell loads its sample in the same update and plays on the
//!   next beat 1: at a pitch drawn from `pitchL..=pitchH` semitones (as a
//!   ratio of the sample's own rate), at volume `vol * volume / 1024 *
//!   score vol / 1024` (the player's volume starts as the score's), and
//!   panned by `LR` and `FB`, or at random when `rand?` (the fifth field)
//!   is set. When it ends, the cell is idle again. A sample that does not
//!   load leaves the cell idle.
//!
//! The original draws on the C library's `rand()`; this player draws on
//! the host player's own random numbers in the same order and range, so
//! a seed replays but the draws are not the original's. Panning: the
//! original places the sound in 3D at `(x, 0, y) * 100`, with `x` from
//! `LR` and `y` from `FB` (each `(v - 512) / 512`, or at random), at its
//! minimum distance of 100; here `x` alone sets a stereo pan (`512 +
//! 512 * x` on the HIT pan scale), which is **unverified**: how
//! DirectSound renders that position was not modeled.
//!
//! Not ported, as no score in the install uses them: fading in and out
//! (`fin`, `fout`: a cell plays and stops at once), the `fast?` cells'
//! beat counter, and the delays and quantising fields (`dly`, `quant`,
//! the third `rand?`, `stereo?`). The player's own fade (vtable +0x28)
//! and tempo change (+0x74), pausing (which follows the game's pause,
//! kinds 21 and 22, not ported), and the second cursor fields of the
//! score data (+0x3c, +0x40, always 0) are left out too.

use crate::fsc::Score;
use std::collections::BTreeSet;

/// The scores the original loads when the sound starts, by ambience mode
/// (`kAmbienceModeBackyard` 0, `Beach` 1, `Park` 2, `Magictown` 3): the
/// directory and file. *(code)* The beach and park scores load when an
/// expansion bit of the configuration is set, the magic one with
/// another; every expansion is installed in the memory image read.
pub const SCORES: [(u32, &str, &str); 4] = [
    (0, "SoundData\\Outdoors", "sim_amb.fsc"),
    (1, "ExpansionShared\\Sound\\Beach", "beach.fsc"),
    (2, "ExpansionShared\\Sound\\Park", "hotdatepark_amb.fsc"),
    (3, "SoundData\\MTOutdoors", "magic_amb.fsc"),
];

/// *(code)* The column (of 13) the cursor takes at each hour: the
/// original's table 0x6ceb1c, indexed by a value it reads from the
/// simulation (taken to be the hour, as the table's 24 entries suggest).
pub const HOUR_COLUMNS: [i32; 24] = [
    12, 12, 12, 12, 12, 1, 2, 3, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 11, 11, 12, 12, 12, 12,
];

/// *(code)* A cell's chance by `prob` (table 0x6cf0f8), out of 32,768
/// times the cells in the window.
pub const CHANCES: [i32; 21] = [
    0, 600, 1500, 3000, 7500, 12_000, 15_000, 18_000, 21_000, 25_500, 32_767, 36_000, 45_000,
    58_000, 67_500, 90_000, 120_000, 210_000, 300_000, 40_000_000, 40_000_000,
];

/// *(code)* `2^(n/12)` for 0 to 12 semitones, the original's doubles
/// (table 0x6cf1b8) bit for bit.
const SEMITONES: [u64; 13] = [
    0x3ff0_0000_0000_0000,
    0x3ff0_f38f_92d9_7431,
    0x3ff1_f59a_c3c7_d030,
    0x3ff3_06fe_0a31_bbfd,
    0x3ff4_28a2_f98d_74c6,
    0x3ff5_5b81_08f0_ebc3,
    0x3ff6_a09e_667f_3a21,
    0x3ff7_f910_d768_d54a,
    0x3ff9_65fe_a53d_6aba,
    0x3ffa_e89f_995a_cc21,
    0x3ffc_823e_074e_c6d1,
    0x3ffe_3437_e710_0c75,
    0x4000_0000_0000_0000,
];

/// *(code)* The original's grid holds at most 64 by 64 cells.
pub const SIDE: usize = 64;

/// A cell's voice: the score's ambience mode and the cell's index.
pub type Key = (u32, usize);

/// A voice to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    pub key: Key,
    /// The sample's install-relative path.
    pub path: String,
    /// The playback rate, frames per second.
    pub rate: u32,
    /// The volume handed to the sound layer, before the master volume.
    pub volume: i32,
    /// A pan on the HIT scale, 0 to 1024.
    pub pan: i32,
    /// Whether it loops until stopped.
    pub looping: bool,
}

/// What the player needs from around it.
pub trait Host {
    /// The next random number, 0 to 32,767, as `rand()` gives.
    fn rand(&mut self) -> i32;
    /// Loads a sample, giving its own rate; `None` when it has none.
    fn load(&mut self, path: &str) -> Option<u32>;
    fn play(&mut self, voice: Voice);
    fn set_volume(&mut self, key: Key, volume: i32);
    fn stop(&mut self, key: Key);
    fn playing(&self, key: Key) -> bool;
}

/// One cell's data, from the score.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Data {
    vol: i32,
    vol_random: bool,
    lr: i32,
    fb: i32,
    pan_random: bool,
    looping: bool,
    loops: i32,
    prob: i32,
    pitch_low: i32,
    pitch_high: i32,
    fast: bool,
    group: i32,
    /// (column, row) pairs.
    exclusions: Vec<(i32, i32)>,
    file: String,
}

/// What a cell is doing (the original's states).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Idle,
    /// Chosen: loads its sample in the next tick.
    Chosen,
    /// Loaded: plays on beat 1.
    Loaded,
    Playing,
    // The original's state 5, fading out, needs `fout`, not ported; its
    // state 3 only goes back to 2 and is never entered.
}

#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    state: State,
    /// Marked to go (+0x15).
    going: bool,
    /// On the beat counter (+0x14; `fast?` cells only).
    synced: bool,
    loops_left: i32,
}

/// The original's 32-bit arithmetic: wrapping products, division that
/// gives 0 instead of trapping.
fn div(a: i32, b: i32) -> i32 {
    a.checked_div(b).unwrap_or(0)
}

fn rem(a: i32, b: i32) -> i32 {
    a.checked_rem(b).unwrap_or(0)
}

/// One score's player.
#[derive(Debug, Clone)]
pub struct Fresh {
    mode: u32,
    directory: String,
    // The score's fields.
    score_volume: i32,
    pri: i32,
    min: i32,
    max: i32,
    dif_x: i32,
    dif_y: i32,
    rows: i32,
    cols: i32,
    score_tempo: i32,
    bpb: i32,
    data: Vec<Option<Data>>,
    cells: Vec<Cell>,
    // The player's.
    volume: i32,
    x: i32,
    y: i32,
    half_rows: i32,
    half_cols: i32,
    in_window: i32,
    slots: i32,
    tempo: i32,
    beat: i32,
    beats: i32,
    first: i32,
    percent: i32,
    started: bool,
    /// The frame the timer counts from.
    timer: u64,
    rate: u32,
}

impl Fresh {
    /// A player of a score, loaded (the original's +0x64 and +0x0c):
    /// `directory` is where its samples are, `rate` the frames per
    /// second its timer counts in. `None` when the score has more than
    /// [`SIDE`] rows or columns.
    pub fn new(mode: u32, score: &Score, directory: &str, rate: u32) -> Option<Self> {
        if score.rows > SIDE || score.columns > SIDE {
            return None;
        }
        // The score data's defaults (0x578ded) stand for fields a short
        // row leaves out, as the original's `sscanf` leaves them.
        let defaults = [1024, 0, 0, 4, 4, 4, 120, 8, 2, 2, 1, 1, 0, 0];
        let field = |at: usize| score.fields.get(at).map_or(defaults[at], |&v| v as i32);
        let data = score
            .cells
            .iter()
            .map(|cell| {
                cell.as_ref().map(|cell| {
                    let f = |at: usize| cell.fields.get(at).map_or(0, |&v| v as i32);
                    Data {
                        vol: f(0),
                        vol_random: f(1) != 0,
                        lr: f(2),
                        fb: f(3),
                        pan_random: f(4) != 0,
                        looping: f(10) != 0,
                        loops: f(9),
                        prob: f(12),
                        pitch_low: f(13),
                        pitch_high: f(14),
                        fast: f(15) != 0,
                        group: f(16),
                        exclusions: cell
                            .exclusions
                            .iter()
                            .map(|&(a, b)| (a as i32, b as i32))
                            .collect(),
                        file: cell.file.clone(),
                    }
                })
            })
            .collect::<Vec<_>>();
        let cells = vec![Cell::default(); data.len()];
        Some(Self {
            mode,
            directory: directory.to_owned(),
            score_volume: field(0),
            pri: field(1),
            min: field(2),
            max: field(3),
            rows: score.rows as i32,
            cols: score.columns as i32,
            score_tempo: field(6),
            bpb: field(7),
            // *(code)* `SelX` is the column half-width, `SelY` the row's.
            half_cols: field(8),
            half_rows: field(9),
            dif_x: field(12),
            dif_y: field(13),
            data,
            cells,
            volume: field(0),
            x: 1,
            y: 1,
            in_window: 0,
            slots: 0,
            tempo: 0,
            beat: 0,
            beats: 1,
            first: 0,
            percent: 100,
            started: false,
            timer: 0,
            rate,
        })
    }

    /// The ambience mode this player is for.
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// Whether it is started.
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// The cursor: row and column.
    pub fn cursor(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    /// The cells not idle.
    pub fn active(&self) -> usize {
        self.cells.iter().filter(|c| c.state != State::Idle).count()
    }

    /// The cells playing.
    pub fn playing(&self) -> usize {
        self.count(|c| c.state == State::Playing) as usize
    }

    fn index(&self, row: i32, col: i32) -> usize {
        (row * self.cols + col) as usize
    }

    fn count(&self, test: impl Fn(&Cell) -> bool) -> i32 {
        self.cells.iter().filter(|c| test(c)).count() as i32
    }

    /// The last row and column of the window (0x578ce9, 0x578cfd).
    fn row_end(&self) -> i32 {
        self.half_rows.wrapping_add(self.x).min(self.rows - 1)
    }

    fn col_end(&self) -> i32 {
        self.half_cols.wrapping_add(self.y).min(self.cols - 1)
    }

    /// *(code)* Counts the window (0x578a59): the cells in it, and its
    /// slots (ungrouped cells and groups).
    fn recount(&mut self) {
        self.in_window = 0;
        self.slots = 0;
        let mut groups = BTreeSet::new();
        for row in self.x.wrapping_sub(self.half_rows).max(0)..=self.row_end() {
            for col in self.y.wrapping_sub(self.half_cols).max(0)..=self.col_end() {
                let Some(data) = &self.data[self.index(row, col)] else {
                    continue;
                };
                if data.group == 0 || groups.insert(data.group) {
                    self.slots += 1;
                }
                self.in_window += 1;
            }
        }
    }

    /// *(code)* vtable +0x30: moves the cursor's row to `x` per mille of
    /// the rows (quantised to the window when `pri` is set), keeping the
    /// window inside the grid where it can.
    pub fn set_x(&mut self, x: i32) {
        let rows = self.rows;
        let mut r = div(rows.wrapping_mul(x), 1000);
        let half = self.half_rows;
        if self.pri != 0 {
            let width = half.wrapping_mul(2).wrapping_add(1);
            r = div(r, width).wrapping_mul(width).wrapping_add(half);
        }
        if r >= rows.wrapping_sub(half) {
            r = rows.wrapping_sub(half).wrapping_sub(1);
        }
        r = r.max(half).min(rows - 1).max(0);
        self.x = r;
        self.recount();
    }

    /// *(code)* vtable +0x34: the same for the column.
    pub fn set_y(&mut self, y: i32) {
        let cols = self.cols;
        let mut c = div(cols.wrapping_mul(y), 1000);
        let half = self.half_cols;
        if self.pri != 0 {
            let width = half.wrapping_mul(2).wrapping_add(1);
            c = div(c, width).wrapping_mul(width).wrapping_add(half);
        }
        if c >= cols.wrapping_sub(half) {
            c = cols.wrapping_sub(half).wrapping_sub(1);
        }
        c = c.max(half).max(0);
        if c >= cols {
            c = cols - 1;
        }
        self.y = c;
        self.recount();
    }

    /// *(code)* vtable +0x70: sets the player's volume, and the volume of
    /// every playing cell.
    pub fn set_volume(&mut self, volume: i32, host: &mut dyn Host) {
        self.volume = volume;
        for at in 0..self.cells.len() {
            if self.cells[at].state == State::Playing {
                let volume = self.cell_volume(at);
                host.set_volume((self.mode, at), volume);
            }
        }
    }

    /// *(code)* 0x579cc5: a cell's volume.
    fn cell_volume(&self, at: usize) -> i32 {
        let vol = self.data[at].as_ref().map_or(0, |d| d.vol);
        let v = vol.wrapping_mul(self.volume) / 1024;
        v.wrapping_mul(self.score_volume) / 1024
    }

    /// *(code)* vtable +0x18: starts the beat from the score's tempo and
    /// moves the cursor to the grid's corner (the score data's cursor
    /// fields, which no file sets). Does nothing when started.
    pub fn start(&mut self, frame: u64) {
        if self.started {
            return;
        }
        self.tempo = self.score_tempo;
        self.timer = frame;
        self.first = if self.bpb > 2 { self.bpb - 1 } else { self.bpb };
        self.beat = self.first;
        self.beats = self.first;
        self.recount();
        self.set_y(0);
        self.set_x(0);
        self.started = true;
    }

    /// *(code)* vtable +0x20 (0x5779c1): stops every cell that is not
    /// idle, and the beat.
    pub fn stop(&mut self, host: &mut dyn Host) {
        if !self.started {
            return;
        }
        for at in 0..self.cells.len() {
            if self.cells[at].state != State::Idle {
                self.stop_cell(at, host);
            }
        }
        self.started = false;
    }

    /// The player's target (vtable +0xb0).
    fn target(&self) -> i32 {
        let base = div(self.percent.wrapping_mul(self.min), 100);
        let cols = if self.cols - 1 == 0 { 1 } else { self.cols - 1 };
        let rows = if self.rows - 1 == 0 { 1 } else { self.rows - 1 };
        let a = div((self.y + 1).wrapping_mul(self.dif_x), cols);
        let b = div((self.x + 1).wrapping_mul(self.dif_y), rows);
        base.wrapping_add(a).wrapping_add(b).min(self.slots)
    }

    /// The player's limit (vtable +0xb4).
    fn limit(&self) -> i32 {
        div(self.max.wrapping_mul(self.percent), 100)
    }

    /// *(code)* The update (vtable +0x14, 0x577ab9), at a frame.
    ///
    /// Two quirks are the original's and kept: the next beat's time is
    /// `(beats - first) * 60000 / tempo` in 32 bits, which wraps after
    /// about 35,800 beats (five hours at 120 beats a minute) and then
    /// never falls due until the score is started again; and the second
    /// pick compares the active count taken before the first, so a beat
    /// can take the score one cell past its limit.
    pub fn update(&mut self, frame: u64, host: &mut dyn Host) {
        let target = self.target();
        let limit = self.limit();
        if !self.started {
            return;
        }
        let elapsed =
            frame.saturating_sub(self.timer).saturating_mul(1000) / u64::from(self.rate.max(1));
        let now = u32::try_from(elapsed).unwrap_or(u32::MAX);
        let due = if self.tempo == 0 {
            u32::MAX
        } else {
            div(
                self.beats.wrapping_sub(self.first).wrapping_mul(60_000),
                self.tempo,
            ) as u32
        };
        if now < due {
            return;
        }
        if now > 86_400_000 {
            self.first = self.bpb;
            self.beat = self.bpb;
            self.beats = self.bpb;
            self.timer = frame;
            return;
        }
        let late = now - due > 7;
        let choosing =
            self.beat == self.bpb.wrapping_sub(1) || self.beat == self.bpb.wrapping_sub(6);
        let mut group = 0;
        let mut prob = 0;
        if choosing
            && self.count(|c| c.state == State::Playing) >= target
            && self.pri != 0
            && let Some(at) = self.pick_going(host)
        {
            self.cells[at].going = true;
            if let Some(data) = &self.data[at] {
                group = data.group;
                if group != 0 {
                    prob = data.prob;
                }
            }
        }
        let going = self.count(|c| c.going);
        let active = self.count(|c| c.state != State::Idle) - going;
        if choosing && self.percent != 0 && active < limit {
            let mut found = self.pick(group, host);
            for _ in 0..2 {
                if found.is_none() && group != 0 && prob == 19 {
                    found = self.pick(group, host);
                }
            }
            if found.is_none() && active < target {
                found = self.pick(group, host);
            }
            if let Some(at) = found {
                self.start_cell(at);
            }
            if active < limit {
                let mut found = self.pick(0, host);
                if found.is_none() && active < target {
                    found = self.pick(0, host);
                }
                if let Some(at) = found {
                    self.start_cell(at);
                }
            }
        }
        for at in 0..self.cells.len() {
            self.tick(at, late, host);
        }
        self.beat = self.beat.wrapping_add(1);
        if self.beat > self.bpb {
            self.beat = 1;
        }
        self.beats = self.beats.wrapping_add(1);
    }

    /// Whether a cell is in the window (vtable +0x84).
    fn in_window(&self, row: i32, col: i32) -> bool {
        row.wrapping_sub(self.x).wrapping_abs() <= self.half_rows
            && col.wrapping_sub(self.y).wrapping_abs() <= self.half_cols
    }

    /// *(code)* A cell's weight (vtable +0x9c, 0x577ec5) for a pick
    /// preferring `group` (0: none).
    fn weight(&mut self, row: i32, col: i32, group: i32, host: &mut dyn Host) -> i32 {
        let at = self.index(row, col);
        let Some(data) = &self.data[at] else {
            return 0;
        };
        if self.cells[at].state != State::Idle || !self.in_window(row, col) {
            return 0;
        }
        let blocks = |cell: &Cell| cell.state != State::Idle && !cell.going;
        if data.group != 0
            && self.data.iter().zip(&self.cells).any(|(other, cell)| {
                other.as_ref().is_some_and(|o| o.group == data.group) && blocks(cell)
            })
        {
            return 0;
        }
        for &(col, row) in &data.exclusions {
            if (0..self.rows).contains(&row)
                && (0..self.cols).contains(&col)
                && blocks(&self.cells[self.index(row, col)])
            {
                return 0;
            }
        }
        let prob = data.prob.clamp(0, 20);
        let own = data.group;
        if let Some(data) = &mut self.data[at] {
            data.prob = prob;
        }
        if prob == 0 {
            return 0;
        }
        let preferred = group != 0 && own == group;
        if prob == 20 {
            return if preferred { 0x65 } else { 0x64 };
        }
        if self.in_window == 0 {
            self.in_window = 10;
        }
        if preferred {
            return 2;
        }
        let chance = CHANCES[prob as usize] / self.in_window;
        i32::from(host.rand() < chance)
    }

    /// *(code)* Picks a cell to start (vtable +0x8c, 0x57808d).
    fn pick(&mut self, group: i32, host: &mut dyn Host) -> Option<usize> {
        let r = host.rand();
        if self.rows <= 0 || self.cols <= 0 {
            return None;
        }
        let col_random = r & 0xff;
        let row_random = (r >> 8) & 0xff;
        let row_low = self.x.wrapping_sub(self.half_rows).max(0);
        let col_low = self.y.wrapping_sub(self.half_cols).max(0);
        let row_end = self.row_end();
        let col_end = self.col_end();
        let row_width = self.half_rows.wrapping_mul(2).wrapping_add(1);
        let col_width = self.half_cols.wrapping_mul(2).wrapping_add(1);
        let row = rem(row_random, row_width)
            .wrapping_sub(self.half_rows)
            .wrapping_add(self.x);
        let col = rem(col_random, col_width)
            .wrapping_sub(self.half_cols)
            .wrapping_add(self.y);
        let start_row = row.max(0).min(self.rows - 1);
        let start_col = col.max(0).min(self.cols - 1);
        let mut best = 0;
        let mut found = None;
        let rows = (start_row..=row_end).chain(row_low..start_row);
        for row in rows {
            for col in (start_col..=col_end).chain(col_low..start_col) {
                let weight = self.weight(row, col, group, host);
                if weight > best {
                    best = weight;
                    found = Some(self.index(row, col));
                }
            }
        }
        found
    }

    /// *(code)* Picks a playing cell to mark to go (vtable +0x90,
    /// 0x5782df): the first outside the window from a random cell, else a
    /// random one inside whose `prob` is not 20.
    fn pick_going(&mut self, host: &mut dyn Host) -> Option<usize> {
        if self.rows <= 0 || self.cols <= 0 {
            return None;
        }
        let r = host.rand();
        let start_row = rem((r >> 8) & 0xff, self.rows);
        let start_col = rem(r & 0xff, self.cols);
        let candidate = |this: &Self, row: i32, col: i32| {
            let cell = &this.cells[this.index(row, col)];
            cell.state == State::Playing && !this.in_window(row, col) && !cell.going
        };
        for row in (start_row..self.rows).chain(0..start_row) {
            for col in (start_col..self.cols).chain(0..start_col) {
                if candidate(self, row, col) {
                    return Some(self.index(row, col));
                }
            }
        }
        let mut inside = Vec::new();
        for row in self.x.wrapping_sub(self.half_rows).max(0)..=self.row_end() {
            for col in self.y.wrapping_sub(self.half_cols).max(0)..=self.col_end() {
                let at = self.index(row, col);
                let cell = &self.cells[at];
                let always = self.data[at].as_ref().is_some_and(|d| d.prob == 20);
                if cell.state == State::Playing && !always && !cell.going {
                    inside.push(at);
                }
            }
        }
        if inside.is_empty() {
            return None;
        }
        // The original draws on the player's own generator here.
        Some(inside[host.rand() as usize % inside.len()])
    }

    /// *(code)* Starts a cell (0x579b12): it loads in its next tick.
    fn start_cell(&mut self, at: usize) {
        let Some(data) = &self.data[at] else {
            return;
        };
        let cell = &mut self.cells[at];
        if cell.state != State::Idle {
            return;
        }
        cell.going = false;
        if data.looping {
            cell.loops_left = data.loops;
        }
        cell.state = State::Chosen;
    }

    /// *(code)* Stops a cell (0x579baa).
    fn stop_cell(&mut self, at: usize, host: &mut dyn Host) {
        self.cells[at].synced = false;
        match self.cells[at].state {
            // A cell with `fout` would fade out here; not ported.
            State::Chosen | State::Loaded | State::Playing => self.release(at, host),
            State::Idle => {}
        }
    }

    /// *(code)* Lets go of a cell's sound (0x579c0d): idle again.
    fn release(&mut self, at: usize, host: &mut dyn Host) {
        host.stop((self.mode, at));
        let cell = &mut self.cells[at];
        cell.synced = false;
        cell.going = false;
        cell.state = State::Idle;
    }

    /// *(code)* A cell's tick on a beat (0x5797ab).
    fn tick(&mut self, at: usize, late: bool, host: &mut dyn Host) {
        let key = (self.mode, at);
        let Some(data) = self.data[at].clone() else {
            return;
        };
        match self.cells[at].state {
            State::Idle => {}
            State::Chosen => {
                let path = format!("{}\\{}", self.directory, data.file);
                if host.load(&path).is_some() {
                    self.cells[at].state = State::Loaded;
                } else {
                    self.release(at, host);
                }
            }
            State::Loaded => {
                if self.beat == self.bpb && data.fast {
                    self.cells[at].synced = true;
                }
                if self.beat == 1 && (!data.fast || !late) {
                    self.cells[at].state = State::Playing;
                    self.play(at, &data, host);
                }
            }
            State::Playing if !data.looping => {
                if !host.playing(key) {
                    self.stop_cell(at, host);
                    self.cells[at].state = State::Idle;
                }
            }
            State::Playing => {
                // Not the original's: a looping sound never ends there,
                // but the host may have dropped it (no room, no frames).
                if !host.playing(key) {
                    self.release(at, host);
                    return;
                }
                let cell = &mut self.cells[at];
                cell.loops_left = cell.loops_left.wrapping_sub(1);
                let left = cell.loops_left;
                if cell.going {
                    if left == 0 {
                        return;
                    }
                    if !data.fast || !late {
                        self.release(at, host);
                        return;
                    }
                }
                let cell = &mut self.cells[at];
                if left == 1 {
                    if data.fast {
                        cell.synced = true;
                    }
                } else if left == 0 {
                    cell.loops_left = data.loops;
                    if !late {
                        cell.synced = false;
                    }
                }
            }
        }
    }

    /// *(code)* Plays a loaded cell's sample (0x5797ab, state 2): pitch,
    /// volume, then pan, drawing random numbers in that order.
    fn play(&mut self, at: usize, data: &Data, host: &mut dyn Host) {
        let path = format!("{}\\{}", self.directory, data.file);
        let Some(own_rate) = host.load(&path) else {
            self.release(at, host);
            return;
        };
        let mut rate = own_rate;
        let (low, high) = (data.pitch_low, data.pitch_high);
        if !(low == 0 && high == 0) && low <= high {
            let span = high.wrapping_sub(low).wrapping_add(1);
            let semitones = rem(host.rand(), span).wrapping_add(low);
            // The original's table ends at 12; past it, the pitch is left.
            if let Some(&bits) = SEMITONES.get(semitones.unsigned_abs() as usize) {
                let ratio = f64::from_bits(bits);
                let base = f64::from(own_rate);
                let scaled = if semitones >= 0 {
                    base * ratio
                } else {
                    base / ratio
                };
                rate = scaled as u32;
            }
        }
        // The randomised volume is only used to fade in (not ported), but
        // its random number is drawn.
        let v = data.vol.wrapping_mul(self.volume) / 1024;
        if data.vol_random && v > 3 {
            host.rand();
        }
        let volume = self.cell_volume(at);
        let pan = self.pan(data, host);
        host.play(Voice {
            key: (self.mode, at),
            path,
            rate,
            volume,
            pan,
            looping: data.looping,
        });
    }

    /// *(code)* A cell's pan (0x579d01), as the x of its 3D position put
    /// on the HIT pan scale (see the module notes): `LR` when not random;
    /// with `rand?`, the centre or either side when `LR` and `FB` are both
    /// centred (the original's four positions, two of them centred), else
    /// either side when `LR` is centred.
    fn pan(&self, data: &Data, host: &mut dyn Host) -> i32 {
        let x = data.lr.wrapping_sub(512);
        if data.pan_random && data.lr == 512 && data.fb == 512 {
            return match host.rand() & 3 {
                2 => 0,
                3 => 1024,
                _ => 512,
            };
        }
        if data.pan_random && data.lr == 512 {
            let side = (host.rand() & 1) * 2 - 1;
            return 512 + side * 512;
        }
        if data.pan_random && data.fb == 512 {
            // Front or back: no stereo difference here.
            host.rand();
        }
        x.wrapping_add(512).clamp(0, 1024)
    }
}
