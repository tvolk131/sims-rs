//! A need's recent history, for the Needs panel's trend (`cAverageHistory`).
//!
//! **What the original does.** The live panel's two sets of need bars,
//! at +0x128 and +0x134, each keep one history per family member:
//!
//! - 0x513a9b and 0x513b41 grow the list to the household's size, with
//!   capacity 10 and limit 5 (0x513aeb, 0x513b87).
//! - When the live panel's update says so (0x51d149..0x51d166), 0x513b41
//!   samples each member's need. That is the person's float at +0x78c plus
//!   4 times the bar's need number (0x4ea6a6 → 0x45d125), rounded half away
//!   from zero (0x513bbf..0x513bdb).
//! - 0x513aa8 points the bar (+0x1b0) at the shown member's history
//!   (0x50c5da) and keeps the member at +0x198. After each sample, while
//!   someone is shown, 0x513b41 sets the meter (0x513c39) and has the bar
//!   read the trend (0x50c66f), which keeps it in its byte +0xde, with
//!   flags +0xdc (drawing) and +0xdd (stop once the fades are over).
//!
//! **How the trend is drawn** ([`TrendBar`]). The bar keeps five arrow
//! slots at +0xf4, 0x34 bytes each, each a colour [`Fade`] (0x50c492:
//! from, to, and a time in ms). 0x50c66f lights |trend| of them, fading
//! over 500 ms from the panel's blue (`0x000052`) to red (`0xff0000`) when
//! the trend is below zero or green (`0x00ff00`) otherwise, and returns
//! the others to the blue. The bar's draw (0x50c2fa) runs while +0xdc is set:
//!
//! - It puts the arrows from the left anchor (+0xe0, x + 16) leftward, 4
//!   apart, for a falling need, and from the right anchor (+0xe8)
//!   rightward otherwise. The needs panel sets both anchors (0x5133ee →
//!   0x50c2cd): (0, 12) and (81, 12) in the need control, from the meter's
//!   place (20, 12) less 0x14 and plus 0x3d.
//! - Each slot draws in its colour at the moment, as a 16-bit pixel
//!   (0x50c3e6..0x50c400), the unlit ones in the blue too.
//! - A slot that has reached full red fades on to `0xd10000`, and full
//!   green to `0x00ca39`, both over 500 ms (0x50c3b6..0x50c3e1). So a
//!   steady arrow is the darker colour.
//! - The arrow (0x50c818) is a 3-wide, 5-high triangle. A falling one
//!   points left (rows x + 2; x + 1..2; x..x + 2; x + 1..2; x + 2) and a
//!   rising one right.
//! - When all five fades are over and +0xdd is set (a zero trend came in),
//!   the bar stops drawing (0x50c457..0x50c47a).
//!
//! **When a need is sampled** ([`NeedsTrend`]). The interface's update
//! message (built by 0x4ce0e4) keeps a [`Pacer`] at +0x200. Each frame,
//! 0x4ce390 adds the millisecond clock (`[0x6e54e0]`) to it and, when it
//! has a whole period, sets flag 1 of the message's flags (+0xc). Flag 8 is
//! set when someone is selected (0x4cf9c4, at 0x4cfa57), and all flags
//! when the update is forced (on going to live mode, 0x4ce432). The
//! control panel passes the message on to the live panel (0x50a8ec, mode 2
//! at 0x50a98d), whose update (0x51ce79) takes `flags & 9` (0x51d05c) and,
//! when it is not 0, samples every need control (0x51d149..0x51d166) once,
//! then again for each further whole period (0x51d173..0x51d183). Only after
//! that does flag 8 run the selection (0x51d23f → 0x51c5a6), which points the
//! bars at the new Sim's histories while the Needs panel is open
//! (0x51c9f4..0x51ca78 → 0x513aa8); opening the panel does the same
//! (0x51d694).
//!
//! The period follows the game's speed (global 16): its setter tells the
//! interface (message 0xc2, flag 0x40), and 0x4cee89 makes the period from
//! the lot's update period: [`needs_period_ms`]. That is 1000 ms at normal
//! speed: one sample per 23.8 lot updates' worth of time.
//!
//! The panel keeps a second set of need bars for pets (+0x134), sampled the
//! same way. Our panel shows the same eight motives for both, so one set is
//! kept.

/// Samples a history keeps (0x513aeb: `push 0xa`).
pub const CAPACITY: usize = 10;
/// The largest trend either way (0x513aeb: `push 5`).
pub const LIMIT: i32 = 5;

/// One member's history of one need (0x4cb4a5): the samples, each with the
/// mean of the history as it stood when the sample came in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    capacity: usize,
    limit: i32,
    samples: Vec<i32>,
    /// +0x10: the mean after each sample. It is not worked out again when
    /// an old sample leaves.
    means: Vec<i32>,
    /// +0x1c: the samples' sum, in 32 bits.
    sum: i32,
}

impl Default for History {
    fn default() -> Self {
        Self::new(CAPACITY, LIMIT)
    }
}

impl History {
    /// An empty history of `capacity` samples whose trend is at most
    /// `limit` either way. A capacity of 0 is taken as 1.
    pub fn new(capacity: usize, limit: i32) -> Self {
        Self {
            capacity: capacity.max(1),
            limit,
            samples: Vec::new(),
            means: Vec::new(),
            sum: 0,
        }
    }

    pub fn samples(&self) -> &[i32] {
        &self.samples
    }

    pub fn means(&self) -> &[i32] {
        &self.means
    }

    /// The mean of the samples (0x4cb58a). It is 0 with none. Otherwise it
    /// is the 32-bit sum over the count, then 0.5 is added, or taken away
    /// below zero, and the result is cut toward zero. So it rounds half
    /// away from zero, and with these magnitudes the x87 quotient is exact
    /// enough that this integer form agrees.
    pub fn mean(&self) -> i32 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum = i64::from(self.sum);
        let count = self.samples.len() as i64;
        let magnitude = (sum.abs() * 2 + count) / (count * 2);
        (if sum < 0 { -magnitude } else { magnitude }) as i32
    }

    /// Adds a sample (0x4cb5d6). A full history first drops its oldest
    /// sample and that sample's mean.
    pub fn push(&mut self, value: i32) {
        if self.samples.len() == self.capacity {
            self.sum = self.sum.wrapping_sub(self.samples.remove(0));
            self.means.remove(0);
        }
        self.samples.push(value);
        self.sum = self.sum.wrapping_add(value);
        self.means.push(self.mean());
    }

    /// The newest mean less the oldest, at most the limit either way
    /// (0x4cb635). The original reads both ends without a check, and its
    /// bar always samples first; with no sample there is no trend here.
    pub fn trend(&self) -> Option<i32> {
        let delta = self.means.last()?.wrapping_sub(*self.means.first()?);
        let low = self.limit.wrapping_neg();
        Some(if delta < low {
            low
        } else if self.limit < delta {
            self.limit
        } else {
            delta
        })
    }
}

/// A need's value as 0x513bbf..0x513bdb samples it: rounded half away from
/// zero, then cut toward zero.
pub fn sample(need: f32) -> i32 {
    let need = f64::from(need);
    (if need < 0.0 { need - 0.5 } else { need + 0.5 }) as i32
}

/// The live panel's blue, where an arrow slot rests (0x50c5da, 0x50c7ab).
pub const BLUE: u32 = 0x00_00_52;
/// A falling need's arrow colour (0x50c722..0x50c72b).
pub const RED: u32 = 0xff_00_00;
/// A rising need's arrow colour (0x50c722..0x50c72b).
pub const GREEN: u32 = 0x00_ff_00;
/// Where a full red arrow fades on to (0x50c3c4).
pub const DARK_RED: u32 = 0xd1_00_00;
/// Where a full green arrow fades on to (0x50c3d9).
pub const DARK_GREEN: u32 = 0x00_ca_39;
/// Milliseconds an arrow takes to change colour (`push 0x1f4`).
pub const FADE_MS: i32 = 500;
/// Arrow slots in a bar (+0xf4, 0x34 bytes each; `push 5` at 0x50c605).
pub const SLOTS: usize = 5;
/// The left anchor in the need control (0x5133ee: the meter's x less
/// 0x14); falling arrows start 16 to its right.
pub const LEFT_ANCHOR: [i32; 2] = [0, 12];
/// The right anchor in the need control (0x5133ee: the meter's x plus
/// 0x3d).
pub const RIGHT_ANCHOR: [i32; 2] = [81, 12];

/// A colour fade (0x50c492): the colours as three floats, red first, the
/// difference, and a ramp from 0 to 1 over a time (0x4733b7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    /// +0: the start colour.
    from: [f32; 3],
    /// +0x18: the end colour less the start (0x49b6df).
    delta: [f32; 3],
    /// +0x2c: the ramp's step per ms, `(1 - 0) / ms` stored as a float.
    rate: f32,
    /// +0x30: the clock at which the ramp is over (`ms + now`, wrapping).
    deadline: u32,
}

impl Fade {
    /// A fade from `from` to `to` (`0xRRGGBB`) over `ms`, starting at the
    /// clock `now` (0x50c492 → 0x4733b7).
    pub fn new(from: u32, to: u32, ms: i32, now: u32) -> Self {
        let channels = |c: u32| {
            [
                ((c >> 16) & 0xff) as f32,
                ((c >> 8) & 0xff) as f32,
                (c & 0xff) as f32,
            ]
        };
        let (from, to) = (channels(from), channels(to));
        Self {
            from,
            delta: std::array::from_fn(|i| to[i] - from[i]),
            // `fld 1.0; fsub 0.0; fidiv ms; fst`: 1/0 is infinite.
            rate: (1.0 / f64::from(ms)) as f32,
            deadline: (ms as u32).wrapping_add(now),
        }
    }

    /// The ramp at `now` (0x473390): 1 from the deadline on (an unsigned
    /// compare), and before it `1 - (deadline - now) × rate`, as a float.
    pub fn ramp(&self, now: u32) -> f32 {
        if now < self.deadline {
            (1.0 - f64::from(self.deadline - now) * f64::from(self.rate)) as f32
        } else {
            1.0
        }
    }

    /// The colour at `now` (0x50c56c): ramp × difference (0x49b704) plus
    /// the start (0x45edf8), each in floats; each channel has 0.5 added and
    /// is cut toward zero (0x636128), and the three are packed red first.
    pub fn colour(&self, now: u32) -> u32 {
        let f = self.ramp(now);
        let channel = |i: usize| {
            let c = self.from[i] + f * self.delta[i];
            // `_ftol` keeps the low 32 bits of a 64-bit integer.
            (f64::from(c) + 0.5).trunc() as i64 as u32
        };
        ((channel(0) << 8 | channel(1)) << 8) | channel(2)
    }

    /// Whether the fade is over at `now` (0x50c434..0x50c43e).
    pub fn done(&self, now: u32) -> bool {
        now >= self.deadline
    }
}

/// What a bar's draw puts on the screen in one frame: which way the
/// arrows point and each slot's colour (`0xRRGGBB`), the first nearest the
/// meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrows {
    pub falling: bool,
    pub colours: [u32; SLOTS],
}

impl Arrows {
    /// The pixels, with their colours, for a need control whose top left
    /// corner is at `control` (0x50c315..0x50c37c, 0x50c818).
    pub fn pixels(&self, control: [i32; 2]) -> Vec<([i32; 2], u32)> {
        let (anchor, step) = if self.falling {
            ([LEFT_ANCHOR[0] + 16, LEFT_ANCHOR[1]], -4)
        } else {
            (RIGHT_ANCHOR, 4)
        };
        let mut pixels = Vec::with_capacity(SLOTS * 9);
        for (slot, &colour) in self.colours.iter().enumerate() {
            let x = control[0] + anchor[0] + step * slot as i32;
            let y = control[1] + anchor[1];
            for (row, (first, last)) in arrow_shape(self.falling).into_iter().enumerate() {
                for dx in first..=last {
                    pixels.push(([x + dx, y + row as i32], colour));
                }
            }
        }
        pixels
    }
}

/// The columns of each of an arrow's five rows (0x50c818): a falling one
/// points left, a rising one right.
pub fn arrow_shape(falling: bool) -> [(i32, i32); 5] {
    if falling {
        [(2, 2), (1, 2), (0, 2), (1, 2), (2, 2)]
    } else {
        [(0, 0), (0, 1), (0, 2), (0, 1), (0, 0)]
    }
}

/// A need bar's trend arrows (the bar made by 0x50c1c0: +0xdc, +0xdd,
/// +0xde, +0xf0 and the slots at +0xf4).
#[derive(Debug, Clone, PartialEq)]
pub struct TrendBar {
    /// +0xdc: the draw shows the arrows.
    drawing: bool,
    /// +0xdd: stop drawing once every fade is over.
    settle: bool,
    /// +0xde: the last trend that was not 0.
    trend: i8,
    /// +0xf0 is not null: the bar has a history.
    history: bool,
    slots: [Fade; SLOTS],
}

impl Default for TrendBar {
    fn default() -> Self {
        let mut bar = Self {
            drawing: false,
            settle: false,
            trend: 0,
            history: false,
            slots: [Fade::new(BLUE, BLUE, 0, 0); SLOTS],
        };
        bar.point(false, 0);
        bar
    }
}

impl TrendBar {
    pub fn drawing(&self) -> bool {
        self.drawing
    }

    pub fn trend(&self) -> i32 {
        i32::from(self.trend)
    }

    /// Points the bar at a history, or at none (0x50c5da): no arrows, every
    /// slot blue at once, and with no history the bar would stop drawing.
    pub fn point(&mut self, history: bool, now: u32) {
        self.history = history;
        self.settle = !history;
        self.drawing = false;
        self.trend = 0;
        self.slots = [Fade::new(BLUE, BLUE, 0, now); SLOTS];
    }

    /// Takes the history's `trend` (0x4cb635) at `now` (0x50c66f). Nothing
    /// happens without a history.
    pub fn update(&mut self, trend: i32, now: u32) {
        if !self.history {
            return;
        }
        let previous = i32::from(self.trend);
        if trend != 0 {
            self.drawing = true;
            // `mov [esi + 0xde], bl`: the trend is at most 5 either way.
            self.trend = trend as i8;
        } else {
            self.settle = true;
        }
        // 0x636cbf is `abs`.
        let (was, lit) = (previous.wrapping_abs(), trend.wrapping_abs());
        let grown = lit.wrapping_sub(was);
        let flip = if trend != 0 && previous != 0 {
            previous.wrapping_mul(trend) < 0
        } else {
            true
        };
        if grown == 0 && !flip {
            return;
        }
        let target = if self.trend >= 0 { GREEN } else { RED };
        let slot = |i: i32| usize::try_from(i).ok().filter(|&i| i < SLOTS);
        if flip {
            // 0x50c7c4..0x50c811: the lit ones fade in from the blue, the
            // rest go back to it at once.
            for i in (0..lit).filter_map(slot) {
                self.slots[i] = Fade::new(BLUE, target, FADE_MS, now);
            }
            for i in (lit.max(0)..SLOTS as i32).filter_map(slot) {
                self.slots[i] = Fade::new(BLUE, BLUE, 0, now);
            }
        } else if grown > 0 {
            // 0x50c743..0x50c780: from the last lit slot (again) outward,
            // each from its colour now.
            let first = if was == 0 { 0 } else { was - 1 };
            for i in (first..lit).filter_map(slot) {
                let colour = self.slots[i].colour(now);
                self.slots[i] = Fade::new(colour, target, FADE_MS, now);
            }
        } else {
            // 0x50c78d..0x50c7c0: from the last lit slot inward, back to
            // the blue.
            for i in (lit..was).rev().filter_map(slot) {
                let colour = self.slots[i].colour(now);
                self.slots[i] = Fade::new(colour, BLUE, FADE_MS, now);
            }
        }
    }

    /// The bar's draw at `now` (0x50c2fa): the arrows to put on the screen,
    /// or none while the bar is not drawing. A slot at full red or green
    /// starts its fade to the darker colour; once all five fades are over,
    /// a bar told to settle stops drawing (after this frame).
    pub fn draw(&mut self, now: u32) -> Option<Arrows> {
        if !self.drawing {
            return None;
        }
        let falling = self.trend < 0;
        let mut colours = [BLUE; SLOTS];
        let mut done = 0;
        for (slot, colour) in self.slots.iter_mut().zip(&mut colours) {
            let c = slot.colour(now);
            if c == RED {
                *slot = Fade::new(RED, DARK_RED, FADE_MS, now);
            } else if c == GREEN {
                *slot = Fade::new(GREEN, DARK_GREEN, FADE_MS, now);
            }
            *colour = c;
            if slot.done(now) {
                done += 1;
            }
        }
        if done == SLOTS && self.settle {
            self.drawing = false;
            self.trend = 0;
            self.settle = false;
        }
        Some(Arrows { falling, colours })
    }

    /// Whether drawing the bar at `now` would still change it: it draws,
    /// and a fade is running, a slot is about to start its darker fade, or
    /// it is about to stop.
    pub fn animating(&self, now: u32) -> bool {
        self.drawing
            && (self.settle
                || self
                    .slots
                    .iter()
                    .any(|s| !s.done(now) || matches!(s.colour(now), RED | GREEN)))
    }
}

/// A period accumulator `{acc, period, last, cap}` (0x4732c0: 0, 100, 0,
/// 0). The cap is 0 for the update message's, so it is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pacer {
    acc: u32,
    period: u32,
    /// The clock at the last add; the original takes a clock of 0 as
    /// none, which our clock, starting at 0, cannot.
    last: Option<u32>,
}

impl Default for Pacer {
    fn default() -> Self {
        Self {
            acc: 0,
            period: 100,
            last: None,
        }
    }
}

impl Pacer {
    /// Sets the period (0x4732d4); what has built up is kept.
    pub fn set_period(&mut self, period: u32) {
        self.period = period;
    }

    pub fn period(&self) -> u32 {
        self.period
    }

    /// Adds the time since the last add (0x4732de). The first add makes a
    /// whole period at once.
    pub fn add(&mut self, now: u32) {
        match self.last {
            Some(last) => self.acc = self.acc.wrapping_add(now.wrapping_sub(last)),
            None => self.acc = self.period,
        }
        self.last = Some(now);
    }

    /// Takes a whole period if there is one (0x473311, an unsigned compare).
    pub fn take(&mut self) -> bool {
        if self.acc >= self.period {
            self.acc -= self.period;
            true
        } else {
            false
        }
    }

    /// Milliseconds after `now` at which an add would make a whole period:
    /// 0 when one is waiting or nothing was added yet.
    pub fn due_in(&self, now: u32) -> u32 {
        let Some(last) = self.last else {
            return 0;
        };
        let acc = self.acc.wrapping_add(now.wrapping_sub(last));
        self.period.saturating_sub(acc)
    }
}

/// The live panel's sampling period at the game's `speed` (0x4cee89): the
/// lot's update period `p` (172 − speed × 0.001 × 171, rounded, as
/// [`sims_sim::speed::period_ms`]), times the float 1/42, times 1000,
/// plus 0.5, cut toward zero, in the x87's precision (exact here in a
/// double). 1000 ms at normal speed, 333 at fast, 48 at ultra and 24 at
/// the idle loop's 1000.
pub fn needs_period_ms(speed: i16) -> u32 {
    let p = f64::from(sims_sim::speed::period_ms(speed));
    let y = p * f64::from(f32::from_bits(0x3cc3_0c31));
    (y * 1000.0 + 0.5) as u32
}

/// The eight need bars of the live panel and their histories, one per
/// household member and need (0x513a9b, 0x513b41), as the live panel's
/// update drives them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NeedsTrend {
    pacer: Pacer,
    /// The histories, by member (`0x4e97dc(i)`) and need. They grow to the
    /// household's size and never shrink.
    histories: Vec<[History; 8]>,
    bars: [TrendBar; 8],
    /// +0x198, while someone is shown: the shown Sim's place among the
    /// members, if one.
    shown: Option<Option<usize>>,
    /// The selection the last frame saw, to raise flag 8.
    selected: Option<u16>,
    /// Whether the last frame was in live mode.
    live: bool,
    /// Whether the last frame was in live mode with the Needs panel open.
    open: bool,
    started: bool,
}

/// What one frame of the live panel's update reads.
#[derive(Debug, Clone, Copy)]
pub struct PanelFrame<'a> {
    /// The millisecond clock.
    pub now: u32,
    /// The game's speed (global 16).
    pub speed: i16,
    /// Whether the panel is in live mode (0x50a98d).
    pub live: bool,
    /// Whether the Needs panel is the open one.
    pub open: bool,
    /// Each household member's motives, in member order.
    pub members: &'a [[f32; 16]],
    /// The selected object, if anyone is selected (flag 8).
    pub selected: Option<u16>,
    /// The selected Sim's place among `members`, if a member.
    pub member: Option<usize>,
    /// The need bars' motive numbers, in the panel's order.
    pub motives: [usize; 8],
}

impl NeedsTrend {
    pub fn bars(&self) -> &[TrendBar; 8] {
        &self.bars
    }

    pub fn histories(&self) -> &[[History; 8]] {
        &self.histories
    }

    /// One frame of 0x4ce390 and the live panel's update (0x51ce79):
    /// add the clock, sample on a whole period, on a new selection, or on
    /// coming to live mode (a forced update, 0x4ce432), then point the bars
    /// at the selected Sim if the Needs panel just opened or the selection
    /// changed while it is open. Returns whether anything was sampled.
    pub fn update(&mut self, frame: &PanelFrame) -> bool {
        if !self.started {
            self.started = true;
            self.pacer.set_period(needs_period_ms(frame.speed));
        }
        self.pacer.add(frame.now);
        let whole = self.pacer.take();
        let forced = frame.live && !self.live;
        let selection = frame.selected != self.selected || forced;
        self.selected = frame.selected;
        let mut sampled = false;
        if frame.live {
            if whole || selection {
                self.sample(frame);
                while self.pacer.take() {
                    self.sample(frame);
                }
                sampled = true;
            }
            if frame.open && (selection || !self.open) {
                self.point(frame);
            }
        }
        // 0x4ce44f handles flag 0x40 after the take, so a new speed's
        // period counts from the next frame.
        self.pacer.set_period(needs_period_ms(frame.speed));
        self.live = frame.live;
        self.open = frame.live && frame.open;
        sampled
    }

    /// 0x513b41 for each bar: grow the histories, sample each member, then
    /// let the bars take the shown member's trend.
    fn sample(&mut self, frame: &PanelFrame) {
        self.grow(frame.members.len());
        for (history, motives) in self.histories.iter_mut().zip(frame.members) {
            for (need, &motive) in frame.motives.iter().enumerate() {
                let value = motives.get(motive).copied().unwrap_or(0.0);
                history[need].push(sample(value));
            }
        }
        if let Some(member) = self.shown {
            for (need, bar) in self.bars.iter_mut().enumerate() {
                let trend = member
                    .and_then(|m| self.histories.get(m))
                    .and_then(|h| h[need].trend())
                    .unwrap_or(0);
                bar.update(trend, frame.now);
            }
        }
    }

    /// 0x513aa8 for each bar: point it at the selected member's history.
    fn point(&mut self, frame: &PanelFrame) {
        self.grow(frame.members.len());
        let member = frame.member.filter(|&m| m < self.histories.len());
        for bar in &mut self.bars {
            bar.point(member.is_some(), frame.now);
        }
        self.shown = frame.selected.map(|_| member);
    }

    /// 0x513b5b..0x513ba2: one more history (capacity 10, limit 5) for each
    /// member past the end.
    fn grow(&mut self, members: usize) {
        if self.histories.len() < members {
            self.histories.resize_with(members, Default::default);
        }
    }

    /// Each bar's draw at `now` (0x50c2fa), while the Needs panel shows.
    pub fn draw(&mut self, now: u32) -> [Option<Arrows>; 8] {
        std::array::from_fn(|i| self.bars[i].draw(now))
    }

    /// Whether a bar is still changing at `now`.
    pub fn animating(&self, now: u32) -> bool {
        self.bars.iter().any(|b| b.animating(now))
    }

    /// Milliseconds after `now` until the next sample is due, once a frame
    /// has run: the first frame's sample needs no frame of its own, as the
    /// first frame takes it whenever it comes.
    pub fn due_in(&self, now: u32) -> Option<u32> {
        self.started.then(|| self.pacer.due_in(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_keeps_each_samples_mean_and_rounds_ties_away_from_zero() {
        let mut h = History::new(3, 5);
        assert_eq!(h.mean(), 0);
        assert_eq!(h.trend(), None);
        for v in [1, 2, 8, -10, -11] {
            h.push(v);
        }
        assert_eq!(h.samples(), [8, -10, -11]);
        // When each came in: 11/3 is 4, 0/3 is 0, -13/3 is -4.
        assert_eq!(h.means(), [4, 0, -4]);
        assert_eq!(h.trend(), Some(-5), "-8, bounded");
        for (values, expected) in [
            (&[0, 1][..], 1),
            (&[0, -1], -1),
            (&[0, 3], 2),
            (&[0, -3], -2),
            (&[1, -1], 0),
        ] {
            let mut h = History::default();
            for &v in values {
                h.push(v);
            }
            assert_eq!(h.mean(), expected, "{values:?}");
        }
    }

    #[test]
    fn the_sum_wraps_in_32_bits() {
        let mut h = History::new(2, 5);
        h.push(i32::MAX);
        h.push(i32::MAX);
        assert_eq!(h.mean(), -1, "the sum wraps to -2");
        h.push(i32::MIN);
        assert_eq!(h.mean(), -1, "-1 / 2 rounds away from zero");
        h.push(0);
        assert_eq!(h.mean(), i32::MIN / 2);
    }

    #[test]
    fn the_trend_is_bounded_by_the_limit() {
        let mut h = History::default();
        h.push(0);
        assert_eq!(h.trend(), Some(0));
        h.push(6);
        // Means 0 then 3.
        assert_eq!(h.trend(), Some(3));
        for _ in 0..8 {
            h.push(60);
        }
        assert_eq!(h.trend(), Some(LIMIT));
        let mut h = History::default();
        h.push(0);
        h.push(-100);
        assert_eq!(h.trend(), Some(-LIMIT));
    }

    #[test]
    fn a_need_samples_rounded_half_away_from_zero() {
        assert_eq!(sample(0.5), 1);
        assert_eq!(sample(-0.5), -1);
        assert_eq!(sample(49.49), 49);
        assert_eq!(sample(-49.5), -50);
        assert_eq!(sample(0.0), 0);
    }

    #[test]
    fn a_fade_ramps_in_floats_and_ends_on_its_colour() {
        let fade = Fade::new(BLUE, RED, FADE_MS, 1000);
        // The float step (0.002 as a float, a little over) leaves the
        // midpoint's red at 127.99..., cut to 127.
        for (now, colour) in [
            (1000, BLUE),
            (1001, 0x01_00_52),
            (1250, 0x7f_00_29),
            (1499, 0xfe_00_00),
            (1500, RED),
            (9000, RED),
        ] {
            assert_eq!(fade.colour(now), colour, "{now}");
        }
        assert!(!fade.done(1499));
        assert!(fade.done(1500));
        assert_eq!(Fade::new(RED, DARK_RED, FADE_MS, 0).colour(250), 0xe8_00_00);
        // No time: the end colour at once.
        let at_once = Fade::new(BLUE, GREEN, 0, 7);
        assert!(at_once.done(7));
        assert_eq!(at_once.colour(7), GREEN);
        // The deadline wraps, and the unsigned compare then ends it early.
        let wrapped = Fade::new(BLUE, RED, FADE_MS, u32::MAX - 100);
        assert!(wrapped.done(u32::MAX - 100));
        assert_eq!(wrapped.colour(u32::MAX - 100), RED);
    }

    fn lit(bar: &mut TrendBar, now: u32) -> Option<(bool, [u32; SLOTS])> {
        bar.draw(now).map(|a| (a.falling, a.colours))
    }

    #[test]
    fn a_bar_lights_its_trend_and_settles_to_the_darker_colour() {
        let mut bar = TrendBar::default();
        bar.update(-3, 0);
        assert!(!bar.drawing(), "no history, no arrows");
        bar.point(true, 0);
        bar.update(-1, 0);
        assert!(bar.drawing());
        assert_eq!(bar.trend(), -1);
        assert_eq!(lit(&mut bar, 0), Some((true, [BLUE; SLOTS])));
        // Full red starts the fade to the darker red as it is drawn.
        assert_eq!(
            lit(&mut bar, 500),
            Some((true, [RED, BLUE, BLUE, BLUE, BLUE]))
        );
        assert!(bar.animating(500));
        assert_eq!(
            lit(&mut bar, 1000),
            Some((true, [DARK_RED, BLUE, BLUE, BLUE, BLUE]))
        );
        assert!(!bar.animating(1000), "steady");

        // Grows: the last lit slot again, then outward, from their colours.
        bar.update(-3, 1000);
        assert_eq!(
            lit(&mut bar, 1500),
            Some((true, [RED, RED, RED, BLUE, BLUE]))
        );
        // The same trend again changes nothing.
        bar.update(-3, 1600);
        // Shrinks: the slots past the new trend fade back to the blue.
        bar.update(-1, 2000);
        assert_eq!(
            lit(&mut bar, 2500),
            Some((true, [DARK_RED, BLUE, BLUE, BLUE, BLUE]))
        );

        // A flip lights the new side from the blue and clears the rest.
        bar.update(2, 3000);
        assert_eq!(lit(&mut bar, 3000), Some((false, [BLUE; SLOTS])));
        assert_eq!(
            lit(&mut bar, 3500),
            Some((false, [GREEN, GREEN, BLUE, BLUE, BLUE]))
        );
        assert_eq!(
            lit(&mut bar, 4000),
            Some((false, [DARK_GREEN, DARK_GREEN, BLUE, BLUE, BLUE]))
        );

        // A zero trend: every slot blue at once, drawn once more, then
        // the bar stops.
        bar.update(0, 5000);
        assert_eq!(bar.trend(), 2, "the last trend that was not 0");
        assert!(bar.animating(5000));
        assert_eq!(lit(&mut bar, 5000), Some((false, [BLUE; SLOTS])));
        assert!(!bar.drawing());
        assert_eq!(bar.trend(), 0);
        assert_eq!(lit(&mut bar, 5001), None);
    }

    #[test]
    fn a_shrink_inside_one_slot_and_a_long_trend_stay_in_the_slots() {
        let mut bar = TrendBar::default();
        bar.point(true, 0);
        bar.update(5, 0);
        assert_eq!(lit(&mut bar, 500), Some((false, [GREEN; SLOTS])));
        bar.update(4, 600);
        assert_eq!(
            lit(&mut bar, 1100),
            Some((
                false,
                [DARK_GREEN, DARK_GREEN, DARK_GREEN, DARK_GREEN, BLUE]
            ))
        );
        // A point puts it out at once.
        bar.point(true, 1200);
        assert!(!bar.drawing());
        assert_eq!(lit(&mut bar, 1200), None);
    }

    #[test]
    fn arrows_point_from_their_anchors() {
        let falling = Arrows {
            falling: true,
            colours: [RED, BLUE, BLUE, BLUE, BLUE],
        };
        let pixels = falling.pixels([520, 521]);
        assert_eq!(pixels.len(), SLOTS * 9);
        let first: Vec<[i32; 2]> = pixels[..9].iter().map(|&(p, _)| p).collect();
        assert_eq!(
            first,
            [
                [538, 533],
                [537, 534],
                [538, 534],
                [536, 535],
                [537, 535],
                [538, 535],
                [537, 536],
                [538, 536],
                [538, 537],
            ]
        );
        assert!(pixels[..9].iter().all(|&(_, c)| c == RED));
        // The fifth slot is 16 to the left.
        assert_eq!(pixels[36].0, [522, 533]);
        let rising = Arrows {
            falling: false,
            colours: [GREEN; SLOTS],
        };
        let pixels = rising.pixels([0, 0]);
        assert_eq!(pixels[0].0, [81, 12]);
        assert_eq!(pixels[5].0, [83, 14], "the tip");
        assert_eq!(pixels[36].0, [97, 12]);
    }

    #[test]
    fn the_pacer_fires_at_once_then_each_period() {
        let mut pacer = Pacer::default();
        assert_eq!(pacer.period(), 100);
        assert_eq!(pacer.due_in(5), 0);
        pacer.add(5);
        assert!(pacer.take());
        assert!(!pacer.take());
        assert_eq!(pacer.due_in(55), 50);
        pacer.add(55);
        assert!(!pacer.take());
        pacer.add(105);
        assert!(pacer.take());
        pacer.add(405);
        assert_eq!((0..5).filter(|_| pacer.take()).count(), 3, "drained");
        // A new period keeps what has built up.
        pacer.add(455);
        pacer.set_period(40);
        assert!(pacer.take());
        assert!(!pacer.take());
    }

    #[test]
    fn the_sampling_period_follows_the_speed() {
        use sims_sim::speed::{FAST, IDLE, NORMAL, ULTRA};
        assert_eq!(needs_period_ms(NORMAL), 1000);
        assert_eq!(needs_period_ms(FAST), 333);
        assert_eq!(needs_period_ms(ULTRA), 48);
        assert_eq!(needs_period_ms(IDLE), 24);
    }

    const MOTIVES: [usize; 8] = [7, 5, 6, 15, 8, 14, 9, 13];

    fn people(hunger: [f32; 2]) -> [[f32; 16]; 2] {
        hunger.map(|h| {
            let mut m = [50.0; 16];
            m[7] = h;
            m
        })
    }

    fn panel<'a>(now: u32, members: &'a [[f32; 16]], selected: Option<u16>) -> PanelFrame<'a> {
        PanelFrame {
            now,
            speed: sims_sim::speed::NORMAL,
            live: true,
            open: true,
            members,
            selected,
            member: selected.map(|s| usize::from(s) - 1),
            motives: MOTIVES,
        }
    }

    #[test]
    fn the_panel_samples_each_period_and_shows_the_selected_trend() {
        let mut trend = NeedsTrend::default();
        let start = people([50.0, 20.0]);
        // The first frame: the pacer's first period, the forced update.
        assert!(trend.update(&panel(10, &start, Some(1))));
        assert_eq!(trend.histories().len(), 2);
        assert_eq!(trend.histories()[1][0].samples(), [20]);
        assert!(!trend.update(&panel(1009, &start, Some(1))), "not yet");
        let hungrier = people([39.6, 20.0]);
        assert!(trend.update(&panel(1010, &hungrier, Some(1))));
        assert_eq!(trend.histories()[0][0].samples(), [50, 40]);
        // Means 50 then 45: a trend of -5.
        let bars = trend.bars();
        assert_eq!(bars[0].trend(), -5);
        assert!(bars[0].drawing());
        assert!(bars[1..].iter().all(|b| !b.drawing()));
        let arrows = trend.draw(1510);
        assert_eq!(
            arrows[0],
            Some(Arrows {
                falling: true,
                colours: [RED; SLOTS]
            })
        );
        assert!(arrows[1..].iter().all(Option::is_none));

        // A long frame samples once per whole period.
        assert!(trend.update(&panel(4010, &hungrier, Some(1))));
        assert_eq!(trend.histories()[0][0].samples(), [50, 40, 40, 40, 40]);

        // A new selection samples first, then points the bars at the new
        // Sim, which puts the arrows out.
        assert!(trend.update(&panel(4500, &hungrier, Some(2))));
        assert_eq!(trend.histories()[0][0].samples().len(), 6);
        assert!(trend.bars().iter().all(|b| !b.drawing()));
        assert_eq!(trend.draw(4500), [None; 8]);
    }

    #[test]
    fn only_live_mode_samples_and_opening_the_panel_points_the_bars() {
        let mut trend = NeedsTrend::default();
        let start = people([50.0, 50.0]);
        let mut frame = panel(0, &start, Some(1));
        frame.live = false;
        assert!(!trend.update(&frame));
        frame.now = 3000;
        assert!(!trend.update(&frame));
        assert!(trend.histories().is_empty());
        // Coming to live mode forces a sample (and a point), then drains
        // the periods left over: outside live mode 0x4ce390 takes one per
        // frame and nothing samples it.
        frame.live = true;
        frame.open = false;
        frame.now = 3100;
        assert!(trend.update(&frame));
        assert_eq!(trend.histories()[0][0].samples(), [50, 50]);
        // The panel closed: the bars point at nobody, so take no trend.
        let falling = people([30.0, 50.0]);
        frame.members = &falling;
        frame.now = 4100;
        assert!(trend.update(&frame));
        assert!(trend.bars().iter().all(|b| !b.drawing()));
        // Opening it points them, and the next sample lights the arrows.
        frame.open = true;
        frame.now = 4200;
        assert!(!trend.update(&frame));
        frame.now = 5100;
        assert!(trend.update(&frame));
        assert_eq!(trend.bars()[0].trend(), -5);
    }
}
