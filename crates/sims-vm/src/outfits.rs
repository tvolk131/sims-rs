//! Person outfit dressing (`cTSPerson::Dress`, `0x58a178`), covering all 61
//! native suit IDs (`0..=60`), NPC flag `0x08` (`0x58a1e4` / `0x58a224`), pet
//! suit reset (`0x58a2c7..0x58a2e5`), cached-guard early exit
//! (`0x58a2ed..0x58a333`), per-slot attachment retention on lookup miss
//! (`0x583559` / `0x583576`), head slot invalidation before rebinding
//! (`0x58d4c1..0x58d51d` → `0x4eb0cc` / `0x4eb0eb`), extra hand pieces
//! (`0x58a453..0x58a675` → `0x58d712..0x58d75e`), clothing-row initialization
//! (`0x409315` → `0x40b7ed` / `0x40bfeb` / `0x40c376` / `0x40c855`), native
//! `cTSHashMap` suit walk (`0x409255`), fuzzy texture matching (`0x40e674` /
//! `0x40e891` → `0x472a25`), career workwear (`0x45ee5f`), matched pajamas
//! (`0x45f034`), skeleton costumes (`0x45f15a`), party costumes (`0x45f452`),
//! and Superstar / Makin' Magic costumes (`0x58a948..0x58d475`).

use crate::Program;
use crate::fault::Trap;
use crate::people::{ResolvedSuit, resolve_suit};
use sims_content::{Catalog, People, SuitTable};
use sims_sim::{BoundSlot, WorkwearCache, World};

/// Person data word indices read/written by `0x58a178`.
const BODY_SUIT: usize = 8;
const VARIANT: usize = 21;
const HAND_LEFT: usize = 22;
const HAND_RIGHT: usize = 23;
const JOB_TYPE: usize = 56;
const JOB_LEVEL: usize = 57;
const AGE: usize = 58;
const GENDER: usize = 65;

/// Encodes a single character into its Windows-1252 byte, preserving ASCII case.
fn cp1252_byte(ch: char) -> Option<u8> {
    let code = ch as u32;
    if code <= 0x7f || (0xa0..=0xff).contains(&code) {
        return Some(code as u8);
    }
    match ch {
        '\u{20ac}' => Some(0x80),
        '\u{0081}' => Some(0x81),
        '\u{201a}' => Some(0x82),
        '\u{0192}' => Some(0x83),
        '\u{201e}' => Some(0x84),
        '\u{2026}' => Some(0x85),
        '\u{2020}' => Some(0x86),
        '\u{2021}' => Some(0x87),
        '\u{02c6}' => Some(0x88),
        '\u{2030}' => Some(0x89),
        '\u{0160}' => Some(0x8a),
        '\u{2039}' => Some(0x8b),
        '\u{0152}' => Some(0x8c),
        '\u{008d}' => Some(0x8d),
        '\u{017d}' => Some(0x8e),
        '\u{008f}' => Some(0x8f),
        '\u{0090}' => Some(0x90),
        '\u{2018}' => Some(0x91),
        '\u{2019}' => Some(0x92),
        '\u{201c}' => Some(0x93),
        '\u{201d}' => Some(0x94),
        '\u{2022}' => Some(0x95),
        '\u{2013}' => Some(0x96),
        '\u{2014}' => Some(0x97),
        '\u{02dc}' => Some(0x98),
        '\u{2122}' => Some(0x99),
        '\u{0161}' => Some(0x9a),
        '\u{203a}' => Some(0x9b),
        '\u{0153}' => Some(0x9c),
        '\u{009d}' => Some(0x9d),
        '\u{017e}' => Some(0x9e),
        '\u{0178}' => Some(0x9f),
        _ => None,
    }
}

/// Encodes `s` into Windows-1252 bytes (falling back to UTF-8 for characters
/// outside Windows-1252), preserving case.
pub(crate) fn encode_cp1252(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for ch in s.chars() {
        if let Some(b) = cp1252_byte(ch) {
            out.push(b);
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    }
    out
}

/// Returns the longest prefix of `s` whose Windows-1252 byte encoding fits
/// within `max_bytes`.
fn truncate_cp1252(s: &str, max_bytes: usize) -> &str {
    let mut used = 0usize;
    for (idx, ch) in s.char_indices() {
        let len = if cp1252_byte(ch).is_some() {
            1
        } else {
            ch.len_utf8()
        };
        if used + len > max_bytes {
            return &s[..idx];
        }
        used += len;
    }
    s
}

/// Parses a `STR# 200` outfit descriptor (`0x5874ca`) into `(suit, texture)`.
///
/// Copies at most 255 Windows-1252 bytes (`0x5874fa`–`0x58752c`) before
/// splitting on `;` (`second_half == false` takes the first half, `true` takes
/// the second half when `;` is present), then splits on `,`:
/// - `suit` (before `,`) is lowercased in ASCII (`0x46f8f7`);
/// - `texture` (after `,`): when it contains `=`, the prefix through `=` keeps
///   its original case while the value after `=` is lowercased (`0x587612`–
///   `0x58765c`); otherwise the whole texture is lowercased (`0x587670`).
pub(crate) fn parse_row_descriptor(row: &str, second_half: bool) -> (String, String) {
    let row = truncate_cp1252(row, 255);
    let half = if second_half {
        match row.find(';') {
            Some(pos) => &row[pos + 1..],
            None => "",
        }
    } else {
        match row.find(';') {
            Some(pos) => &row[..pos],
            None => row,
        }
    };
    let (suit_raw, tex_raw) = match half.find(',') {
        Some(pos) => (&half[..pos], Some(&half[pos + 1..])),
        None => (half, None),
    };
    let suit = suit_raw.to_ascii_lowercase();
    let texture = match tex_raw {
        None => String::new(),
        Some(rest) => match rest.find('=') {
            Some(eq) => {
                let mut out = String::with_capacity(rest.len());
                out.push_str(&rest[..=eq]);
                out.push_str(&rest[eq + 1..].to_ascii_lowercase());
                out
            }
            None => rest.to_ascii_lowercase(),
        },
    };
    (suit, texture)
}

/// Parses an ASCII integer prefix the way CRT `atoi` does (`0x58a712`, `0x40cbdd`).
fn atoi_age(s: &str) -> i32 {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let neg = if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
        true
    } else {
        if i < bytes.len() && bytes[i] == b'+' {
            i += 1;
        }
        false
    };
    let mut acc: i32 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        acc = acc
            .wrapping_mul(10)
            .wrapping_add(i32::from(bytes[i] - b'0'));
        i += 1;
    }
    if neg { acc.wrapping_neg() } else { acc }
}

/// Native gender code returned by `0x40ccac`: `0` male, `1` female, `2`
/// dogmale, `3` dogfemale, `4` catmale, `5` catfemale, `-1` unknown/missing.
fn profile_gender_code(gender: Option<&str>) -> i32 {
    let Some(g) = gender else {
        return -1;
    };
    const GENDERS: [&str; 6] = [
        "male",
        "female",
        "dogmale",
        "dogfemale",
        "catmale",
        "catfemale",
    ];
    GENDERS
        .iter()
        .position(|name| name.eq_ignore_ascii_case(g))
        .map_or(-1, |idx| idx as i32)
}

/// One node of the native pattern matcher (`0x472657`, `0x4726c5`, `0x47275d`),
/// stored as Windows-1252 bytes.
#[derive(Debug, Clone)]
enum PatternStep {
    Literal(Vec<u8>),
    Choice(Vec<u8>, Vec<u8>),
    Skip(usize),
}

/// Prefix pattern automaton (`0x472657`–`0x4729b2`), returning the maximum
/// matched prefix byte length on Windows-1252 ASCII-case-insensitive comparison.
#[derive(Debug, Clone, Default)]
struct Pattern {
    steps: Vec<PatternStep>,
}

impl Pattern {
    fn push_literal(&mut self, s: &str) {
        self.steps.push(PatternStep::Literal(encode_cp1252(s)));
    }

    fn push_choice(&mut self, a: &str, b: &str) {
        // 0x4726c5 -> 0x4727f1: the NFA always starts at node 0, so a choice
        // appended to an empty pattern can only enter its first branch `a`;
        // `b` is unreachable without a preceding node.
        if self.steps.is_empty() {
            self.steps.push(PatternStep::Literal(encode_cp1252(a)));
        } else {
            self.steps
                .push(PatternStep::Choice(encode_cp1252(a), encode_cp1252(b)));
        }
    }

    fn push_skip(&mut self, count: usize) {
        self.steps.push(PatternStep::Skip(count));
    }

    /// `0x4727f1`: returns the longest matched prefix length (in Windows-1252
    /// bytes) across all paths, or `0` if no path completes all steps.
    fn match_len(&self, cand: &[u8]) -> usize {
        if self.steps.is_empty() {
            return 0;
        }
        let mut best = 0usize;
        self.walk(0, 0, cand, &mut best);
        best
    }

    fn walk(&self, step_idx: usize, pos: usize, cand: &[u8], best: &mut usize) {
        let Some(step) = self.steps.get(step_idx) else {
            if pos > *best {
                *best = pos;
            }
            return;
        };
        let try_lit = |bytes: &[u8], best: &mut usize| {
            if pos + bytes.len() <= cand.len()
                && cand[pos..pos + bytes.len()].eq_ignore_ascii_case(bytes)
            {
                self.walk(step_idx + 1, pos + bytes.len(), cand, best);
            }
        };
        match step {
            PatternStep::Literal(lit) => try_lit(lit, best),
            PatternStep::Choice(a, b) => {
                try_lit(a, best);
                try_lit(b, best);
            }
            PatternStep::Skip(n) => {
                if pos + *n <= cand.len() {
                    self.walk(step_idx + 1, pos + *n, cand, best);
                }
            }
        }
    }
}

/// Fuzzy best-ratio selector (`0x472a25`), comparing `matched / cp1252_len` in
/// double precision against the stored `f32` score with the native epsilon at
/// `0x667878` (`0.0001_f32`).
#[derive(Default)]
struct BestMatch<'a> {
    name: &'a str,
    score: f32,
    matched: usize,
}

impl<'a> BestMatch<'a> {
    fn consider(&mut self, candidate: &'a str, cand_len: usize, matched: usize) {
        if matched == 0 || cand_len == 0 {
            return;
        }
        let fraction = matched as f64 / cand_len as f64;
        let old = f64::from(self.score);
        if fraction > old
            || ((fraction - old).abs() <= f64::from(0.0001_f32) && matched > self.matched)
        {
            self.name = candidate;
            self.score = fraction as f32;
            self.matched = matched;
        }
    }
}

/// Returns the cached `0x409255` (`0x6e48a0`) walk of `AvatarRegistry`'s
/// appearance hash table, building and caching it on first non-empty use
/// (`0x40922f` retries whenever the cached vector is empty) from
/// `program.people.suit_table` (or `program.ordered_wardrobe` / `People` sets
/// in synthetic fixtures) plus any runtime-added `.bcf` sibling appearances
/// (`World::runtime_added_suits`). Shared identically by generated dressing
/// (`0x40e622`) and `Make New Character` (`0x40976a`, `0x40a436`).
pub(crate) fn suit_scan_order(program: &Program, world: &mut World) -> Vec<String> {
    if let Some(cached) = &world.suit_scan_cache
        && !cached.is_empty()
    {
        return cached.clone();
    }
    let people = &program.people;
    if people.suit_table.is_empty()
        && world.runtime_added_suits.is_empty()
        && let Some(Ok(ordered)) = &program.ordered_wardrobe
    {
        let scan: Vec<String> = ordered
            .suits()
            .iter()
            .map(|b| sims_formats::strings::decode(b, 1))
            .collect();
        if !scan.is_empty() {
            world.suit_scan_cache = Some(scan.clone());
        }
        return scan;
    }
    let mut table: SuitTable = if !people.suit_table.is_empty() {
        people.suit_table.clone()
    } else if let Some(Ok(ordered)) = &program.ordered_wardrobe {
        let mut t = SuitTable::default();
        for name in ordered.suits() {
            if !t.contains(name) {
                t.insert_name(name.clone());
            }
        }
        t
    } else {
        // Fallback for synthetic tests that populate `People` sets directly
        // without running `AvatarRegistry::load` or supplying `ordered_wardrobe`.
        let mut t = SuitTable::default();
        for name in people
            .eager_suits
            .iter()
            .chain(people.indexed_suits.keys())
            .chain(people.suits.iter())
            .chain(people.head_suits.iter())
        {
            let key = encode_cp1252(name);
            if !t.contains(&key) {
                t.insert_name(key);
            }
        }
        t
    };
    for name in &world.runtime_added_suits {
        let key = encode_cp1252(name);
        if !table.contains(&key) {
            table.insert_name(key);
        }
    }
    let scan = table.scan_names();
    if !scan.is_empty() {
        world.suit_scan_cache = Some(scan.clone());
    }
    scan
}

/// Returns the texture stem list (`0x40895a` / `0x6e48b0`), falling back to
/// `ordered_wardrobe.textures()` in synthetic fixtures where `people.textures`
/// is empty.
fn texture_list(program: &Program) -> Vec<String> {
    if !program.people.textures.is_empty() {
        return program.people.textures.clone();
    }
    if let Some(Ok(ordered)) = &program.ordered_wardrobe {
        return ordered
            .textures()
            .iter()
            .map(|b| sims_formats::strings::decode(b, 1))
            .collect();
    }
    Vec::new()
}

/// Extracts up to 3 raw Windows-1252 bytes starting at byte offset 6
/// (`0x40a727`, `0x40a840`, `0x40a880`, `0x40a8c0`, `0x40a900`, `0x40aa20`,
/// `0x40aa60`, `0x40ab55`), preserving case for bytewise `strcmp` (`0x634d50`).
fn cp1252_body_type_raw(row: &str) -> Vec<u8> {
    let bytes = encode_cp1252(row);
    bytes
        .get(6..)
        .map(|rest| rest[..rest.len().min(3)].to_vec())
        .unwrap_or_default()
}

/// Extracts up to 3 bytes of body type starting at Windows-1252 byte offset 6
/// (`0x40b84a`, `0x40e59f` → `0x46f775`, `0x40afeb`), stopping at the end of
/// the string and lowercased in ASCII.
fn cp1252_body_type(suit: &str) -> String {
    let raw = cp1252_body_type_raw(suit);
    sims_formats::strings::decode(&raw, 1).to_ascii_lowercase()
}

/// Expands a `$g`/`$b`/`$c` template (`0x40e477`) into a [`Pattern`].
fn expand_template_pattern(
    template: &str,
    gender_code: i32,
    body_type: &str,
    skin_3: &str,
) -> Pattern {
    let mut pattern = Pattern::default();
    let mut chars = template.chars().peekable();
    let mut lit = String::new();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            if !lit.is_empty() {
                pattern.push_literal(&lit);
                lit.clear();
            }
            if let Some(next) = chars.next() {
                match next {
                    'g' | 'G' => {
                        let g = if gender_code == 1 { "f" } else { "m" };
                        pattern.push_choice(g, "u");
                    }
                    'b' | 'B' => {
                        pattern.push_literal(body_type);
                    }
                    'c' | 'C' => {
                        pattern.push_literal(skin_3);
                    }
                    _ => {}
                }
            }
            continue;
        }
        lit.push(ch);
    }
    if !lit.is_empty() {
        pattern.push_literal(&lit);
    }
    pattern
}

/// Matches a template against the cached `0x409255` appearance table
/// (`0x40e5ce` / `0x40e9b2`), returning the first entry with a non-zero match.
/// Calls `suit_scan_order` (`0x40e60e` → `0x40922f`) even when `template` is
/// empty so the shared `0x6e48a0` scan freezes identically to native code.
fn match_registered_suit(
    program: &Program,
    world: &mut World,
    template: &str,
    gender_code: i32,
    body_type: &str,
    skin_3: &str,
) -> String {
    let pattern = expand_template_pattern(template, gender_code, body_type, skin_3);
    let scan = suit_scan_order(program, world);
    for name in &scan {
        let bytes = encode_cp1252(name);
        if pattern.match_len(&bytes) > 0 {
            return name.clone();
        }
    }
    String::new()
}

/// Matches a texture template against `People::textures` (`0x40e674` for body,
/// `0x40e891` for head), capping `base_tex` to the 63-byte `0x472ed2`/`0x472ee2`
/// field before slicing the optional `0x40b75a` suffix (`base_tex[17..]`) even
/// when `template` is empty (`0x40e6dc`–`0x40e6f1`, `0x40e8f9`–`0x40e90e`),
/// scoring via `0x472a25`, and capping the formatted descriptor to `max_len`
/// Windows-1252 bytes (`63` for body at `0x40e760`, `255` for head at `0x58c7c9`).
#[allow(clippy::too_many_arguments)]
fn match_registered_texture(
    program: &Program,
    template: &str,
    gender_code: i32,
    body_type: &str,
    skin_3: &str,
    base_tex: &str,
    prefix: &str,
    max_len: usize,
) -> String {
    let mut pattern = expand_template_pattern(template, gender_code, body_type, skin_3);
    // 0x472ed2 / 0x472ee2 parse `base_tex` into a 63-byte field before
    // `0x40b75a` takes `strlen("BODY=") + 12 = 17` bytes off the front.
    let base_bytes = encode_cp1252(base_tex);
    let base_63 = &base_bytes[..base_bytes.len().min(63)];
    let suffix = if base_63.len() > 17 {
        sims_formats::strings::decode(&base_63[17..], 1)
    } else {
        String::new()
    };
    pattern.push_choice(&suffix, "");
    let textures = texture_list(program);
    let mut best = BestMatch::default();
    for tex in &textures {
        let bytes = encode_cp1252(tex);
        let matched = pattern.match_len(&bytes);
        best.consider(tex, bytes.len(), matched);
    }
    if best.name.is_empty() {
        String::new()
    } else {
        let out = format!("{prefix}{}", best.name);
        truncate_cp1252(&out, max_len).to_owned()
    }
}

/// Expands a hand template (`0x40e795`), replacing `$h` with `'O'`/`'P'`/`'C'`
/// for gesture 0/1/2 and `$g` with `'f'`/`'m'`, prefixing `"HAND="` on BOTH
/// the `$g`-absent (`0x40e7f3`) and `$g`-present (`0x40e856`) paths.
fn expand_hand_template(template: &str, gesture: i16, gender_code: i32) -> String {
    let ch = match gesture {
        1 => 'P',
        2 => 'C',
        _ => 'O',
    };
    let pos_h = template.find("$h");
    let pos_g = template.find("$g");
    let Some(ph) = pos_h else {
        return template.to_owned();
    };
    let mut out = String::new();
    if pos_g.is_none() {
        out.push_str("HAND=");
    }
    out.push_str(&template[..ph]);
    out.push(ch);
    out.push_str(&template[ph + 2..]);
    if let Some(pg) = pos_g {
        let g = if gender_code == 1 { 'f' } else { 'm' };
        let split = pg.min(out.len());
        let mut s = String::from("HAND=");
        s.push_str(&out[..split]);
        s.push(g);
        if split + 2 <= out.len() {
            s.push_str(&out[split + 2..]);
        }
        out = s;
    }
    out
}

/// Formats `"{suit},BODY={tex}"` truncated to the 63-byte Windows-1252
/// `StackString<64>` capacity (`0x40be06`, `0x40c2ef`, `0x40c74a`, `0x40cb56`).
fn format_generated_body_row(suit: &str, tex: &str) -> String {
    let out = format!("{suit},BODY={tex}");
    truncate_cp1252(&out, 63).to_owned()
}

/// Fuzzy-matches `(suit_pat, tex_pat)` against `scan` and `textures` via
/// `0x472a25` and formats `"{suit},BODY={tex}"` capped at 63 Windows-1252 bytes.
fn match_body_pair(
    suit_pat: &Pattern,
    tex_pat: &Pattern,
    scan: &[String],
    textures: &[String],
) -> String {
    let mut best_suit = BestMatch::default();
    for name in scan {
        let bytes = encode_cp1252(name);
        best_suit.consider(name, bytes.len(), suit_pat.match_len(&bytes));
    }
    let mut best_tex = BestMatch::default();
    for tex in textures {
        let bytes = encode_cp1252(tex);
        best_tex.consider(tex, bytes.len(), tex_pat.match_len(&bytes));
    }
    format_generated_body_row(best_suit.name, best_tex.name)
}

fn read_character_row(
    world: &World,
    guid: u32,
    profile: Option<&sims_content::Profile>,
    idx: usize,
) -> Option<String> {
    if let Some(s) = world
        .characters
        .made
        .get(&guid)
        .and_then(|m| m.strings.get(&idx))
    {
        return Some(s.clone());
    }
    if let Some(s) = world.made_outfits.get(&guid).and_then(|m| m.get(&idx)) {
        return Some(s.clone());
    }
    match idx {
        1 => profile.and_then(|p| p.body.clone()),
        2 => profile.and_then(|p| p.head.clone()),
        12 => profile.and_then(|p| p.gender.clone()),
        13 => profile.and_then(|p| p.age.clone()),
        14 => profile.and_then(|p| p.skin.clone()),
        n => profile.and_then(|p| p.suit_rows.get(&n).cloned()),
    }
}

/// Runs `0x40b7ed` (adult formal 30, swim 31, pajama 32).
fn build_adult_fsl(
    gender_ch: &str,
    adult_body_type: &str,
    skin_norm: &str,
    scan: &[String],
    textures: &[String],
) -> (String, String, String) {
    let make_pats = |kind: &str| {
        let mut sp = Pattern::default();
        sp.push_literal(kind);
        sp.push_literal("100");
        sp.push_literal(gender_ch);
        sp.push_literal("a");
        sp.push_literal(adult_body_type);
        let mut tp = sp.clone();
        tp.push_literal(skin_norm);
        (sp, tp)
    };
    let (f_s, f_t) = make_pats("f");
    let (s_s, s_t) = make_pats("s");
    let (l_s, l_t) = make_pats("l");
    (
        match_body_pair(&f_s, &f_t, scan, textures),
        match_body_pair(&s_s, &s_t, scan, textures),
        match_body_pair(&l_s, &l_t, scan, textures),
    )
}

/// Runs `0x40c376` (child swim 31, pajama 32; row 30 becomes `"ADDED"`).
fn build_child_sl(
    gender_ch: &str,
    raw_body_type: &str,
    skin_norm: &str,
    scan: &[String],
    textures: &[String],
) -> (String, String) {
    let make_pats = |kind: &str| {
        let mut sp = Pattern::default();
        sp.push_literal(kind);
        sp.push_literal("100");
        sp.push_literal(gender_ch);
        sp.push_literal("c");
        sp.push_literal(raw_body_type);
        let mut tp = sp.clone();
        tp.push_literal(skin_norm);
        (sp, tp)
    };
    let (s_s, s_t) = make_pats("s");
    let (l_s, l_t) = make_pats("l");
    (
        match_body_pair(&s_s, &s_t, scan, textures),
        match_body_pair(&l_s, &l_t, scan, textures),
    )
}

/// Runs `0x40bfeb` (adult high-fashion row 34).
fn build_adult_hf(
    gender_ch: &str,
    adult_body_type: &str,
    skin_norm: &str,
    scan: &[String],
    textures: &[String],
) -> String {
    let mut sp = Pattern::default();
    sp.push_literal("h");
    sp.push_literal("5");
    sp.push_skip(2);
    sp.push_literal(gender_ch);
    sp.push_literal("a");
    sp.push_literal(adult_body_type);
    let mut tp = sp.clone();
    tp.push_literal(skin_norm);
    match_body_pair(&sp, &tp, scan, textures)
}

/// Runs `0x40c855` (winter row 33 for adult or child).
fn build_winter(
    gender_ch: &str,
    is_child: bool,
    adult_body_type: &str,
    skin_norm: &str,
    scan: &[String],
    textures: &[String],
) -> String {
    let mut sp = Pattern::default();
    sp.push_literal("w");
    sp.push_literal("100");
    sp.push_literal(gender_ch);
    if is_child {
        sp.push_literal("cchd");
    } else {
        sp.push_literal("a");
        sp.push_literal(adult_body_type);
    }
    let mut tp = sp.clone();
    tp.push_literal(skin_norm);
    match_body_pair(&sp, &tp, scan, textures)
}

/// Runs `0x409315` (deterministic clothing-row initialization: `0x40b7ed`,
/// `0x40bfeb`, `0x40c376`, `0x40c855`) before generated-outfit matching and
/// before `Make New Character` edits (`0x44afc0`, `0x44b26e`, `0x44b27b`),
/// populating missing formal (30), swim (31), pajama (32), winter (33), and
/// high-fashion (34) rows into `World::made_outfits` for human profiles with a
/// non-empty body row 1.
pub(crate) fn init_clothing_rows(
    program: &Program,
    world: &mut World,
    guid: u32,
    profile: Option<&sims_content::Profile>,
) {
    let row_empty = |world: &World, idx: usize| -> bool {
        read_character_row(world, guid, profile, idx).is_none_or(|s| s.is_empty())
    };
    let Some(body_row) = read_character_row(world, guid, profile, 1).filter(|s| !s.is_empty())
    else {
        return;
    };
    let gender_code = profile_gender_code(read_character_row(world, guid, profile, 12).as_deref());
    if !(0..=1).contains(&gender_code) {
        return;
    }
    let gender_ch = if gender_code == 1 { "f" } else { "m" };
    // 0x40937b -> 0x40cbdd copies at most 15 Windows-1252 bytes before `atoi`.
    let age_str = read_character_row(world, guid, profile, 13).unwrap_or_default();
    let is_child = atoi_age(truncate_cp1252(&age_str, 15)) <= 17;
    let (body_suit, _) = parse_row_descriptor(&body_row, false);
    let raw_body_type = cp1252_body_type(&body_suit);
    let adult_body_type = if matches!(raw_body_type.as_str(), "skn" | "fit" | "fat") {
        raw_body_type.as_str()
    } else {
        "fit"
    };
    let skin_raw = read_character_row(world, guid, profile, 14).unwrap_or_else(|| "???".to_owned());
    let skin_3: String = skin_raw.chars().take(3).collect();
    let skin_norm = if skin_3.eq_ignore_ascii_case("lgt")
        || skin_3.eq_ignore_ascii_case("med")
        || skin_3.eq_ignore_ascii_case("drk")
    {
        skin_3.as_str()
    } else {
        "lgt"
    };
    let textures = texture_list(program);

    if !is_child {
        // 0x40938b -> 0x40b7ed: adult formal (30), swim (31), pajama (32).
        if row_empty(world, 30) {
            let scan = suit_scan_order(program, world);
            let (formal, swim, pajama) =
                build_adult_fsl(gender_ch, adult_body_type, skin_norm, &scan, &textures);
            let entry = world.made_outfits.entry(guid).or_default();
            entry.insert(30, formal);
            entry.insert(31, swim);
            entry.insert(32, pajama);
        }
        // 0x4093be -> 0x40bfeb: adult high fashion (34).
        if row_empty(world, 34) {
            let scan = suit_scan_order(program, world);
            let hf = build_adult_hf(gender_ch, adult_body_type, skin_norm, &scan, &textures);
            world.made_outfits.entry(guid).or_default().insert(34, hf);
        }
    } else {
        // 0x4093d8 -> 0x40c376: child "ADDED" on row 30, swim (31), pajama (32).
        if row_empty(world, 30) {
            let scan = suit_scan_order(program, world);
            let (swim, pajama) =
                build_child_sl(gender_ch, &raw_body_type, skin_norm, &scan, &textures);
            let entry = world.made_outfits.entry(guid).or_default();
            entry.insert(30, "ADDED".to_owned());
            entry.insert(31, swim);
            entry.insert(32, pajama);
        }
        // 0x40941b: child "ADDED" on row 34.
        if row_empty(world, 34) {
            world
                .made_outfits
                .entry(guid)
                .or_default()
                .insert(34, "ADDED".to_owned());
        }
    }
    // 0x409452 -> 0x40c855: winter (33) for both adult and child.
    if row_empty(world, 33) {
        let scan = suit_scan_order(program, world);
        let winter = build_winter(
            gender_ch,
            is_child,
            adult_body_type,
            skin_norm,
            &scan,
            &textures,
        );
        world
            .made_outfits
            .entry(guid)
            .or_default()
            .insert(33, winter);
    }
}

/// How `0x40a588`'s body check (`0x40a5a1`, `0x40a6a6`, `0x40ab7f..0x40acbc`)
/// resolved row 1 before clothing validation (`0x40acbc`) and `0x40af5b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BodyCommitState {
    /// `body_idx != -1`: run the `0x40a6a6..0x40ab7b` body-type comparisons
    /// against the matched body row.
    Matched,
    /// `body_idx == -1` and both body and head lists were non-empty:
    /// `0x40abf9..0x40ac50` replaced row 1 with index 0 and invalidated all
    /// relevant clothing indices (`f/s/l/w/h` for adults, `s/l/w` for children).
    ReplacedWithFallback,
    /// `body_idx == -1` and an empty body or head list sent execution to
    /// `0x40ac57` (clearing rows `1, 2, 15..=18`): `0x40a6b4` skipped body-type
    /// comparisons and did not invalidate inherited clothing indices.
    ClearedEmptyList,
}

/// Runs `0x40a588` (post-edit clothing commit for rows 30..=34) followed by
/// `0x40af5b` (rebuilding naked row 15, swim row 16, formal row 27, and hand
/// rows 17..=22, 28..=29) at the end of `Make New Character` human creation
/// (`0x44b185`) and negative-age copying (`0x44b308`).
pub(crate) fn commit_character_clothing(
    program: &Program,
    world: &mut World,
    guid: u32,
    profile: Option<&sims_content::Profile>,
    body_state: BodyCommitState,
    pre_clear_body_row: Option<&str>,
) {
    let gender_code = profile_gender_code(read_character_row(world, guid, profile, 12).as_deref());
    if !(0..=1).contains(&gender_code) {
        return;
    }
    let gender_ch = if gender_code == 1 { "f" } else { "m" };
    let gender_byte = gender_ch.as_bytes()[0];
    let age_str = read_character_row(world, guid, profile, 13).unwrap_or_default();
    let is_child = atoi_age(truncate_cp1252(&age_str, 15)) <= 17;
    let age_byte = if is_child { b'c' } else { b'a' };
    let body_row = read_character_row(world, guid, profile, 1).unwrap_or_default();
    let (body_suit, body_tex) = parse_row_descriptor(&body_row, false);
    let raw_body_type = cp1252_body_type(&body_suit);
    let adult_body_type = if matches!(raw_body_type.as_str(), "skn" | "fit" | "fat") {
        raw_body_type.as_str()
    } else {
        "fit"
    };
    let skin_raw = read_character_row(world, guid, profile, 14).unwrap_or_else(|| "???".to_owned());
    let skin_3: String = skin_raw.chars().take(3).collect();
    let skin_norm = if skin_3.eq_ignore_ascii_case("lgt")
        || skin_3.eq_ignore_ascii_case("med")
        || skin_3.eq_ignore_ascii_case("drk")
    {
        skin_3.as_str()
    } else {
        "lgt"
    };
    let skin_bytes = encode_cp1252(&skin_3.to_ascii_lowercase());
    let scan = suit_scan_order(program, world);
    let textures = texture_list(program);

    // Build exact `0x408bf5` wardrobe list for clothing kind (`f`/`s`/`l`/`w`/`h`).
    // `0x472e7e–0x472eb7` store separate 63-byte suit and `BODY=<tex>` fields
    // joined at `0x472f41` as `"{suit_63},{desc_63}"` (up to 127 bytes).
    let build_list = |kind: u8| -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for suit in &scan {
            let sb = encode_cp1252(suit);
            if sb.len() < 9
                || !sb[0].eq_ignore_ascii_case(&kind)
                || !(sb[4].eq_ignore_ascii_case(&gender_byte) || sb[4].eq_ignore_ascii_case(&b'u'))
                || !sb[5].eq_ignore_ascii_case(&age_byte)
            {
                continue;
            }
            let suit_63 = truncate_cp1252(suit, 63).to_owned();
            let mut prefix = sb[..9].to_ascii_lowercase();
            prefix.extend_from_slice(&skin_bytes);
            for tex in &textures {
                let tb = encode_cp1252(tex);
                if tb.len() >= prefix.len() && tb[..prefix.len()].eq_ignore_ascii_case(&prefix) {
                    let desc_full = format!("BODY={tex}");
                    let desc_63 = truncate_cp1252(&desc_full, 63).to_owned();
                    let row = format!("{suit_63},{desc_63}");
                    out.push((suit_63.clone(), desc_63, row));
                }
            }
        }
        out
    };
    // `0x472ec4 -> 0x5874ca -> 0x472f92`: cap raw row at 255 CP1252 bytes,
    // strip `;...`, split on `,`, and compare 63-byte suit and texture
    // descriptor fields case-insensitively.
    let find_in_list = |list: &[(String, String, String)], raw: Option<&str>| -> Option<usize> {
        let raw = raw?;
        if raw.is_empty() {
            return None;
        }
        let row_255 = truncate_cp1252(raw, 255);
        let before_semi = row_255.split(';').next().unwrap_or(row_255);
        let (want_suit, want_tex) = before_semi.split_once(',')?;
        let want_suit_63 = truncate_cp1252(want_suit, 63);
        let want_tex_63 = truncate_cp1252(want_tex, 63);
        list.iter().position(|(s, t, _)| {
            s.eq_ignore_ascii_case(want_suit_63) && t.eq_ignore_ascii_case(want_tex_63)
        })
    };

    // Snapshot inherited clothing rows before writing `"ADDED"` / fallbacks.
    let row_30 = read_character_row(world, guid, profile, 30);
    let row_31 = read_character_row(world, guid, profile, 31);
    let row_32 = read_character_row(world, guid, profile, 32);
    let row_33 = read_character_row(world, guid, profile, 33);
    let row_34 = read_character_row(world, guid, profile, 34);

    let formal_list = build_list(b'f');
    let swim_list = build_list(b's');
    let pajama_list = build_list(b'l');
    let winter_list = build_list(b'w');
    let hf_list = if !is_child {
        build_list(b'h')
    } else {
        Vec::new()
    };

    let (mut f_idx, mut s_idx, mut l_idx, mut w_idx, mut h_idx) = if !is_child {
        (
            find_in_list(&formal_list, row_30.as_deref()),
            find_in_list(&swim_list, row_31.as_deref()),
            find_in_list(&pajama_list, row_32.as_deref()),
            find_in_list(&winter_list, row_33.as_deref()),
            find_in_list(&hf_list, row_34.as_deref()),
        )
    } else {
        // 0x40a667..0x40a6a3: child commits write "ADDED" to rows 30 and 34
        // and leave f_idx = 0, h_idx = 0.
        let entry = world.made_outfits.entry(guid).or_default();
        entry.insert(30, "ADDED".to_owned());
        entry.insert(34, "ADDED".to_owned());
        (
            Some(0),
            find_in_list(&swim_list, row_31.as_deref()),
            find_in_list(&pajama_list, row_32.as_deref()),
            find_in_list(&winter_list, row_33.as_deref()),
            Some(0),
        )
    };

    // 0x40a6a6..0x40ab7b: when `body_idx != -1`, compare each matched clothing
    // row's raw body type (offset 6, up to 3 bytes) against row 1's raw body
    // type using bytewise `strcmp` (`0x634d50`, case-sensitive). When
    // `body_idx == -1` and replaced with index 0 (`0x40abf9..0x40ac34`),
    // invalidate all relevant clothing indices instead.
    match body_state {
        BodyCommitState::Matched => {
            let validation_body_row = pre_clear_body_row.unwrap_or(&body_row);
            let body_bt = cp1252_body_type_raw(validation_body_row);
            if !is_child {
                if matches!(body_bt.as_slice(), b"skn" | b"fit" | b"fat") {
                    if f_idx.is_some()
                        && cp1252_body_type_raw(row_30.as_deref().unwrap_or("")) != body_bt
                    {
                        f_idx = None;
                    }
                    if s_idx.is_some()
                        && cp1252_body_type_raw(row_31.as_deref().unwrap_or("")) != body_bt
                    {
                        s_idx = None;
                    }
                    if l_idx.is_some()
                        && cp1252_body_type_raw(row_32.as_deref().unwrap_or("")) != body_bt
                    {
                        l_idx = None;
                    }
                    if h_idx.is_some()
                        && cp1252_body_type_raw(row_34.as_deref().unwrap_or("")) != body_bt
                    {
                        h_idx = None;
                    }
                }
            } else if body_bt.as_slice() == b"chd" {
                if s_idx.is_some()
                    && cp1252_body_type_raw(row_31.as_deref().unwrap_or("")) != body_bt
                {
                    s_idx = None;
                }
                if l_idx.is_some()
                    && cp1252_body_type_raw(row_32.as_deref().unwrap_or("")) != body_bt
                {
                    l_idx = None;
                }
            }
            if matches!(body_bt.as_slice(), b"skn" | b"fit" | b"fat" | b"chd")
                && w_idx.is_some()
                && cp1252_body_type_raw(row_33.as_deref().unwrap_or("")) != body_bt
            {
                w_idx = None;
            }
        }
        BodyCommitState::ReplacedWithFallback => {
            if !is_child {
                f_idx = None;
                s_idx = None;
                l_idx = None;
                w_idx = None;
                h_idx = None;
            } else {
                s_idx = None;
                l_idx = None;
                w_idx = None;
            }
        }
        BodyCommitState::ClearedEmptyList => {}
    }

    // 1. 0x40acbc..0x40aeb4: rows 30, 31, 32 (formal / swim / pajama).
    // Note `0x40acd4..0x40acf9` checks ALL THREE list counts (`formal`, `swim`,
    // `pajama`) before branching on `is_child` (`0x40acff`), clearing rows
    // 30..=32 to `""` if any of the three lists is empty.
    if f_idx.is_none() || s_idx.is_none() || l_idx.is_none() {
        let entry = world.made_outfits.entry(guid).or_default();
        if formal_list.is_empty() || swim_list.is_empty() || pajama_list.is_empty() {
            entry.insert(30, String::new());
            entry.insert(31, String::new());
            entry.insert(32, String::new());
        } else if !is_child {
            if f_idx.is_none() && s_idx.is_none() && l_idx.is_none() {
                let (formal, swim, pajama) =
                    build_adult_fsl(gender_ch, adult_body_type, skin_norm, &scan, &textures);
                entry.insert(30, formal);
                entry.insert(31, swim);
                entry.insert(32, pajama);
            } else {
                entry.insert(30, formal_list[f_idx.unwrap_or(0)].2.clone());
                entry.insert(31, swim_list[s_idx.unwrap_or(0)].2.clone());
                entry.insert(32, pajama_list[l_idx.unwrap_or(0)].2.clone());
            }
        } else if s_idx.is_none() && l_idx.is_none() {
            let (swim, pajama) =
                build_child_sl(gender_ch, &raw_body_type, skin_norm, &scan, &textures);
            entry.insert(30, "ADDED".to_owned());
            entry.insert(31, swim);
            entry.insert(32, pajama);
        } else {
            entry.insert(31, swim_list[s_idx.unwrap_or(0)].2.clone());
            entry.insert(32, pajama_list[l_idx.unwrap_or(0)].2.clone());
        }
    }

    // 2. 0x40aec4..0x40aee8: row 33 (winter).
    if w_idx.is_none() {
        let val = if winter_list.is_empty() {
            String::new()
        } else {
            build_winter(
                gender_ch,
                is_child,
                adult_body_type,
                skin_norm,
                &scan,
                &textures,
            )
        };
        world.made_outfits.entry(guid).or_default().insert(33, val);
    }

    // 3. 0x40aef8..0x40af1c: row 34 (adult high fashion).
    if h_idx.is_none() {
        let val = if hf_list.is_empty() {
            String::new()
        } else {
            build_adult_hf(gender_ch, adult_body_type, skin_norm, &scan, &textures)
        };
        world.made_outfits.entry(guid).or_default().insert(34, val);
    }

    // 4. 0x40af51 -> 0x40af5b: rebuild naked (15), swim (16), formal (27), and
    //    hand rows (28, 29, 17, 18, 21, 22, 19, 20) from the clone's new body,
    //    gender, age, and skin tone.
    let base_bytes = encode_cp1252(&body_tex);
    let base_63 = &base_bytes[..base_bytes.len().min(63)];
    let suffix = if base_63.len() > 17 {
        sims_formats::strings::decode(&base_63[17..], 1)
    } else {
        String::new()
    };
    let make_other_suit_pat = |first: &str| {
        let mut p = Pattern::default();
        p.push_literal(first);
        p.push_choice("U", gender_ch);
        p.push_literal(&raw_body_type);
        p
    };
    let make_other_tex_pat = |first: &str| {
        let mut p = make_other_suit_pat(first);
        p.push_literal(&skin_3);
        p.push_choice(&suffix, "");
        p
    };
    let find_best = |pat: &Pattern, items: &[String]| -> String {
        let mut best = BestMatch::default();
        for item in items {
            let b = encode_cp1252(item);
            best.consider(item, b.len(), pat.match_len(&b));
        }
        best.name.to_owned()
    };
    let naked_suit = find_best(&make_other_suit_pat("N"), &scan);
    let naked_tex = find_best(&make_other_tex_pat("N"), &textures);
    let swim_tex_raw = find_best(&make_other_tex_pat("U"), &textures);
    let swim_tex = if swim_tex_raw.is_empty() && is_child {
        naked_tex.as_str()
    } else {
        swim_tex_raw.as_str()
    };
    let formal_suit = find_best(&make_other_suit_pat("F"), &scan);
    let formal_tex = find_best(&make_other_tex_pat("F"), &textures);

    let entry = world.made_outfits.entry(guid).or_default();
    entry.insert(15, format_generated_body_row(&naked_suit, &naked_tex));
    entry.insert(16, format_generated_body_row(&naked_suit, swim_tex));
    entry.insert(27, format_generated_body_row(&formal_suit, &formal_tex));

    let hand_gender = if is_child {
        "u"
    } else if gender_code == 1 {
        "f"
    } else {
        "m"
    };
    const HAND_SPECS: [(usize, &str, &str); 8] = [
        (28, "L", "G"),
        (29, "R", "G"),
        (17, "L", "O"),
        (18, "R", "O"),
        (21, "L", "C"),
        (22, "R", "C"),
        (19, "L", "P"),
        (20, "R", "P"),
    ];
    for (row_idx, side, pose) in HAND_SPECS {
        let mut hp = Pattern::default();
        hp.push_literal("H");
        hp.push_choice(gender_ch, "U");
        hp.push_choice(side, "A");
        hp.push_literal(pose);
        hp.push_choice(&skin_3, "");
        hp.push_choice(&suffix, "");
        let best_hand = find_best(&hp, &textures);
        // 0x40b5e3..0x40b657: clears the buffer and unconditionally writes it
        // to `row_idx`, clearing stale inherited hand rows on a miss.
        let value = if best_hand.is_empty() {
            String::new()
        } else {
            let formatted = format!("H{hand_gender}{side}{pose},HAND={best_hand}");
            truncate_cp1252(&formatted, 63).to_owned()
        };
        entry.insert(row_idx, value);
    }
}

/// Costume table entry for Superstar & Makin' Magic suits (`19..=60`).
struct CostumeSpec {
    body: Option<&'static str>,
    body_tex: Option<&'static str>,
    head: Option<&'static str>,
    head_tex: Option<&'static str>,
    hand_left: Option<&'static str>,
    hand_right: Option<&'static str>,
}

const fn body_only(body: &'static str, body_tex: &'static str) -> CostumeSpec {
    CostumeSpec {
        body: Some(body),
        body_tex: Some(body_tex),
        head: None,
        head_tex: None,
        hand_left: None,
        hand_right: None,
    }
}

const fn full_costume(
    body: &'static str,
    body_tex: &'static str,
    head: &'static str,
    head_tex: &'static str,
    hand: &'static str,
) -> CostumeSpec {
    CostumeSpec {
        body: Some(body),
        body_tex: Some(body_tex),
        head: Some(head),
        head_tex: Some(head_tex),
        hand_left: Some(hand),
        hand_right: Some(hand),
    }
}

fn costume_spec(suit: i16, variant: i16) -> Option<CostumeSpec> {
    match suit {
        19 => Some(body_only(
            "superstar$gaFit_movieactor1",
            "superstar$gaFit$c_movieactor1",
        )),
        20 => Some(body_only(
            "superstar$gaFit_movieactor2",
            "superstar$gaFit$c_movieactor2",
        )),
        21 => Some(body_only(
            "superstar$gaFit_movieactor3",
            "superstar$gaFit$c_movieactor3",
        )),
        22 => Some(body_only(
            "Superstar$gAfit_Doctor",
            "Superstar$gAfit$c_Doctor",
        )),
        23 => Some(body_only(
            "Superstar$gAfit_Relative",
            "Superstar$gAfit$c_Relative",
        )),
        24 => Some(body_only("Runway$gAfit_Coffee", "Runway$gAfit$c_Coffee")),
        25 => Some(body_only("Runway$gAfit_Floral", "Runway$gAfit$c_Floral")),
        26 => Some(body_only(
            "Runway$gAfit_Architectural",
            "Runway$gAfit$c_Architectural",
        )),
        27 => Some(body_only("Runway$gAfit_Formal1", "Runway$gAfit$c_Formal1")),
        28 => Some(body_only("Runway$gAfit_Formal2", "Runway$gAfit$c_Formal2")),
        29 => Some(body_only("Runway$gAfit_Formal3", "Runway$gAfit$c_Formal3")),
        30 => Some(body_only(
            "Runway$gAfit_Lingerie1",
            "Runway$gAfit$c_Lingerie1",
        )),
        31 => Some(body_only(
            "Runway$gAfit_Lingerie2",
            "Runway$gAfit$c_Lingerie2",
        )),
        32 => Some(body_only(
            "Runway$gAfit_Lingerie3",
            "Runway$gAfit$c_Lingerie3",
        )),
        33 => Some(body_only(
            "Superstar$gAfit_JumpSuit",
            "Superstar$gAfit$c_JumpSuit",
        )),
        34 => Some(body_only(
            "Superstar$gAfit_WetSuit",
            "Superstar$gAfit$c_WetSuit",
        )),
        35 => Some(body_only(
            "Superstar$gAFit_BackupDancerDance",
            "Superstar$gAFit$c_BackupDancerDance",
        )),
        36 => Some(body_only(
            "Superstar$gAfit_BackupDancerPop",
            "Superstar$gAfit$c_BackupDancerPop",
        )),
        37 => Some(body_only(
            "Superstar$gAfit_BackupDancerRock",
            "Superstar$gAfit$c_BackupDancerRock",
        )),
        38 => Some(CostumeSpec {
            body: Some("MagicMAFat_BeeKeeper"),
            body_tex: Some("MagicMAFat_BeeKeeper"),
            head: None,
            head_tex: None,
            hand_left: Some("HUA$h_BeeKeeper"),
            hand_right: Some("HUA$h_BeeKeeper"),
        }),
        39 => Some(full_costume(
            "Magic$gAFit_BlueGenie",
            "BMagic$gAFit_BlueGenie",
            "CMagic$gA_BlueGenie",
            "CMagic$gAFit_BlueGenie",
            "H$gA$h_BlueGenie",
        )),
        40 => Some(full_costume(
            "MagicMAFit_Minotaur",
            "BMagicMAFit_Minotaur",
            "CMagicMAFit_Minotaur",
            "CMagicMAFit_Minotaur",
            "HMA$h_Minotaur",
        )),
        41 => Some(body_only(
            "Magic$gAFit_Spookshow",
            "Magic$gAFit$c_Spookshow",
        )),
        42 => Some(body_only("Magic$gAFit_SideShow", "Magic$gAFit$c_SideShow")),
        43 => Some(full_costume(
            "MagicFAFit_Vampire",
            "MagicFAFit_Vampire",
            "CMagicFAFit_Vampire",
            "CMagicFAFit_Vampire",
            "HFA$h_Vampire",
        )),
        44 => Some(full_costume(
            "Magic$gAFit_Werewolf",
            "Magic$gAFit_Werewolf",
            "CMagic$gAFit_Werewolf",
            "CMagic$gAFit_Werewolf",
            "HMA$h_Werewolf",
        )),
        45 => Some(full_costume(
            "MagicMAFit_Headless",
            "MagicMAFit_Headless",
            "CMagicMA_Headless",
            "CMagicMA_Headless",
            "HUA$h_Headless",
        )),
        46 => Some(full_costume(
            "MagicMAFat_Troll",
            "MagicMAFat_Troll",
            "CMagicMAFat_Troll",
            "CMagicMAFat_Troll",
            "HUA$h_Troll",
        )),
        47 => Some(full_costume(
            "MagicMAFat_Hunchback",
            "MagicMAFat_Hunchback",
            "CMagicMAFat_Hunchback",
            "CMagicMAFat_Hunchback",
            "HUA$h_Hunchback",
        )),
        48 => Some(full_costume(
            "Magic$gAFit_Sorcerer",
            "Magic$gAFit_Sorcerer",
            "CMagic$gAFit_Sorcerer",
            "CMagic$gAFit_Sorcerer",
            "HMA$h_Sorcerer",
        )),
        49 => Some(full_costume(
            "MagicFAFit_Witch",
            "MagicFAFit_Witch",
            "CMagicFAFit_Witch",
            "CMagicFAFit_Witch",
            "HUA$h_Hunchback",
        )),
        50 => Some(full_costume(
            "MagicFAFat_Ogress",
            "BMagicFAFat_Ogress",
            "CMagicFAFat_Ogress",
            "CMagicFAFat_Ogress",
            "HFA$h_Ogress",
        )),
        51 => Some(full_costume(
            "MagicFASkn_Crone",
            "BMagicFASkn_Crone",
            "CMagicFASkn_Crone",
            "CMagicFASkn_Crone",
            "HFA$h_Crone",
        )),
        52 => Some(full_costume(
            "MagicFASkn_Nymph",
            "BMagicFASkn_Nymph",
            "CMagicFASkn_Nymph",
            "CMagicFASkn_Nymph",
            "HUA$h_Nymph",
        )),
        53 => Some(full_costume(
            "MagicMCChd_Goblin",
            "MagicMCChd_Goblin",
            "CMagicMCChd_Goblin",
            "CMagicMCChd_Goblin",
            "HUC$h_Goblin",
        )),
        54 => Some(full_costume(
            "MagicFCChd_WizardGirl",
            "MagicFCChd_WizardGirl",
            "CMagicFC_WizardGirl",
            "CMagicFC_WizardGirl",
            "HFC$h_WizardGirl",
        )),
        55 | 56 => match variant {
            0 => Some(full_costume(
                "MagicMAFit_MummyFull",
                "MagicMAFit_Mummy",
                "CMagicMA_Mummy",
                "CMagicMA_Mummy",
                "HUA$h_Mummy",
            )),
            1 => Some(full_costume(
                "MagicMAFit_MummyFull",
                "MagicMAFit_Mummy",
                "CMagicMA_Headless",
                "CMagicMA_Headless",
                "HUA$h_Mummy",
            )),
            2 => Some(full_costume(
                "MagicMAFit_MummyTorso",
                "MagicMAFit_Mummy",
                "CMagicMA_Headless",
                "CMagicMA_Headless",
                "HUA$h_Mummy",
            )),
            3 => Some(CostumeSpec {
                body: Some("MagicMAFit_MummyLegs"),
                body_tex: Some("MagicMAFit_Mummy"),
                head: Some("CMagicMA_Headless"),
                head_tex: Some("CMagicMA_Headless"),
                hand_left: Some("nohandl"),
                hand_right: Some("nohandr"),
            }),
            _ => Some(full_costume(
                "MagicMAFit_MummyFull",
                "MagicMAFit_Mummy",
                "CMagicMA_Mummy",
                "CMagicMA_Mummy",
                "HUA$h_Mummy",
            )),
        },
        57 => Some(CostumeSpec {
            body: None,
            body_tex: None,
            head: Some("CMagicMAFat_Donkey"),
            head_tex: Some("CMagicMAFat_Donkey"),
            hand_left: None,
            hand_right: None,
        }),
        58 => Some(full_costume(
            "MagicMASkn_Vampire",
            "MagicMASkn_Vampire",
            "CMagicMASkn_Vampire",
            "CMagicMASkn_Vampire",
            "HMA$h_Vampire",
        )),
        59 => Some(body_only(
            "MagicMAFit_PrinceCharming",
            "MagicMAFitlgt_PrinceCharming",
        )),
        60 => Some(full_costume(
            "MagicMCChd_Skeleton",
            "MagicMCChd_Skeleton",
            "CMagicMCChd_Skeleton",
            "CMagicMCChd_Skeleton",
            "GUA$h_SkeletonChild",
        )),
        _ => None,
    }
}

/// Binds `(suit, texture)` onto `slot` following `0x583559`:
/// - empty `suit` returns immediately without modifying `slot` (`0x583569`);
/// - non-empty `suit` (including literal `"none"`) is passed byte-for-byte to
///   `0x5a044d` (`resolve_suit`, `0x58356f`);
/// - `Found(has_head)` replaces `slot`;
/// - `Missing` returns before `0x58359c` detaches the previous attachment;
/// - `Unknown` (decode error prevented knowing whether `suit` exists/binds
///   `"HEAD"`) preserves the slot's uncertainty (`has_head: None`).
fn bind_slot(
    people: &People,
    world: &mut World,
    slot: &mut Option<BoundSlot>,
    suit: &str,
    texture: &str,
) {
    if suit.is_empty() {
        return;
    }
    match resolve_suit(people, world, suit) {
        ResolvedSuit::Found(has_head) => {
            *slot = Some(BoundSlot {
                suit: suit.to_owned(),
                texture: texture.to_owned(),
                has_head: Some(has_head),
            });
        }
        ResolvedSuit::Missing => {
            // 0x583576: lookup miss returns without detaching the previous slot.
        }
        ResolvedSuit::Unknown => {
            *slot = Some(BoundSlot {
                suit: suit.to_owned(),
                texture: texture.to_owned(),
                has_head: None,
            });
        }
    }
}

/// Whether `existing` differs from `(suit, texture)` per `0x58a12f`:
/// returns `true` when `existing` is empty or either `suit` or `texture`
/// differs (byte-exact per `0x59384d`).
fn slot_differs(existing: &Option<BoundSlot>, suit: &str, texture: &str) -> bool {
    match existing {
        None => true,
        Some(bound) => bound.suit != suit || bound.texture != texture,
    }
}

/// Executes `cTSPerson::Dress(force)` (`0x58a178`) on `person_id` in `world`.
///
/// Called from:
/// - `person_factory::construction_reset` (`0x588930`, `force = false`)
/// - `people::read_appearance_if_stale` (`0x5888d3` → `0x588930`, `force = false`)
/// - `frame::draw_person` (`0x590cb8`, `force = false`)
/// - portrait rendering in `ui_effect` / `dialog` (`0x4e7801`, `force = true`)
pub(crate) fn dress_person(
    catalog: &Catalog,
    program: &Program,
    world: &mut World,
    person_id: u16,
    force: bool,
) -> Result<(), Trap> {
    let Some(object) = world.objects.get(&person_id) else {
        return Ok(());
    };
    let guid = object.guid;
    let Some(person) = object.person.as_ref() else {
        return Ok(());
    };
    let word = |idx: usize| person.data.get(idx).copied().unwrap_or(0);
    let raw_suit = word(BODY_SUIT);
    let variant = word(VARIANT);
    let hand_left_word = word(HAND_LEFT);
    let hand_right_word = word(HAND_RIGHT);
    let job_type = word(JOB_TYPE);
    let job_level = word(JOB_LEVEL);
    let age_word = word(AGE);
    let gender_word = word(GENDER);

    let def_flags = crate::interpreter::object_definition(catalog, world, guid)
        .and_then(|def| program.definition(def.resource, def.chunk))
        .map(|w| w[102])
        .unwrap_or(0);

    let template = world.characters.definition(guid);
    let profile = program.people.profiles.get(&template);
    let made_strings = world.characters.made.get(&guid).map(|m| m.strings.clone());
    let made_outfit = world.made_outfits.get(&guid).cloned();

    let raw_row = |idx: usize| -> Option<String> {
        if let Some(s) = made_strings.as_ref().and_then(|m| m.get(&idx)) {
            return Some(s.clone());
        }
        if let Some(s) = made_outfit.as_ref().and_then(|rows| rows.get(&idx)) {
            return Some(s.clone());
        }
        match idx {
            1 => profile.and_then(|p| p.body.clone()),
            2 => profile.and_then(|p| p.head.clone()),
            12 => profile.and_then(|p| p.gender.clone()),
            13 => profile.and_then(|p| p.age.clone()),
            14 => profile.and_then(|p| p.skin.clone()),
            n => profile.and_then(|p| p.suit_rows.get(&n).cloned()),
        }
    };
    let row_desc = |idx: usize| -> (String, String) {
        raw_row(idx)
            .map(|r| parse_row_descriptor(&r, false))
            .unwrap_or_default()
    };

    // 0x58a1e4: definitions with flag `0x08` bypass the normal dressing path.
    if (def_flags & 8) != 0 {
        // 0x58a1f8: `raw_suit <= 0` jumps directly to `0x58d78a`, leaving all
        // attachment slots, `data[8]`, `.ndx` registries, and portraits alone.
        if raw_suit <= 0 {
            return Ok(());
        }
        // 0x58a224..0x58a2c2: `raw_suit > 0` skips rebinding whenever
        // `last_dressed[0] == raw_suit` (even when `force == true`!), and
        // otherwise updates `last_dressed[0]` and binds `STR# 200` row
        // `raw_suit` onto the primary body slot (`0x583559`) without touching
        // the head slot or calling `0x4eb0cc`.
        let mut outfit = world.person_outfits.remove(&person_id).unwrap_or_default();
        if outfit.last_dressed[0] == i32::from(raw_suit) {
            world.person_outfits.insert(person_id, outfit);
            return Ok(());
        }
        outfit.last_dressed[0] = i32::from(raw_suit);
        let (suit, tex) = row_desc(raw_suit as usize);
        bind_slot(&program.people, world, &mut outfit.body, &suit, &tex);
        world.person_outfits.insert(person_id, outfit);
        return Ok(());
    }

    // 0x58a2c7..0x58a2e5: pets reset live `person.data[8] = 0`.
    let suit = if (gender_word & 6) != 0 && raw_suit != 0 {
        if let Some(slot) = world
            .objects
            .get_mut(&person_id)
            .and_then(|o| o.person.as_mut())
            .and_then(|p| p.data.get_mut(BODY_SUIT))
        {
            *slot = 0;
        }
        world.routing.unknown_words.remove(&(person_id, BODY_SUIT));
        0
    } else {
        raw_suit
    };

    let dress_key = [
        i32::from(suit),
        i32::from(variant),
        i32::from(hand_left_word),
        i32::from(hand_right_word),
    ];

    let mut outfit = world.person_outfits.remove(&person_id).unwrap_or_default();

    // 0x58a2ed..0x58a333: when `force == false` and `[ebx + 0x1b0..0x1bc]`
    // match `(data[8], data[21], data[22], data[23])`, exit immediately.
    if !force && outfit.last_dressed == dress_key {
        world.person_outfits.insert(person_id, outfit);
        return Ok(());
    }
    outfit.last_dressed = dress_key;

    // 0x40ccac: gender code from `STR# 200` row 12.
    let gender_code = profile_gender_code(raw_row(12).as_deref());
    let (body_row_suit, body_row_tex) = row_desc(1);
    let (_, head_row_tex) = row_desc(2);
    let body_type = cp1252_body_type(&body_row_suit);
    let skin_3: String = raw_row(14)
        .unwrap_or_else(|| "???".to_owned())
        .chars()
        .take(3)
        .collect();
    let is_formal_child = atoi_age(raw_row(13).as_deref().unwrap_or("")) <= 17;
    let is_word_child = age_word != 0 && age_word < 18;

    // Default head (row 2) and hand extras (words 22 & 23, `0x58a453..0x58a675`).
    let (mut head_suit, mut head_tex) = row_desc(2);
    let (mut extra_left_suit, mut extra_left_tex) = match hand_left_word {
        1 => row_desc(19),
        2 => row_desc(21),
        3 => ("nohandl".to_owned(), "nohand".to_owned()),
        4 => row_desc(25),
        _ => row_desc(17),
    };
    let (mut extra_right_suit, mut extra_right_tex) = match hand_right_word {
        1 => row_desc(20),
        2 => row_desc(22),
        3 => ("nohandr".to_owned(), "nohand".to_owned()),
        4 => row_desc(26),
        _ => row_desc(18),
    };
    let mut body_suit = String::new();
    let mut body_tex = String::new();
    let mut body_alt_suit = String::new();

    match suit {
        0 | ..=-1 | 61.. => {
            (body_suit, body_tex) = row_desc(1);
        }
        1 => {
            (body_suit, body_tex) = row_desc(15);
        }
        2 => {
            // 0x58a6d6: checks raw row 31 presence BEFORE parsing.
            if raw_row(31).is_some_and(|r| !r.is_empty()) {
                (body_suit, body_tex) = row_desc(31);
            } else {
                (body_suit, body_tex) = row_desc(16);
            }
        }
        3 => {
            // 0x45ee5f: career workwear cached by `(job_type, job_level)` on
            // `cXPerson` (`[esi + 0xb2c..0xbc8]`), choosing the CARR string
            // using live `person.data[65] & 1`.
            if let Some(cached) = outfit
                .workwear_cache
                .as_ref()
                .filter(|c| c.job_type == job_type && c.job_level == job_level)
            {
                body_suit = cached.body_suit.clone();
                body_tex = cached.body_tex.clone();
                body_alt_suit = cached.body_alt_suit.clone();
            } else {
                let level_idx = if is_word_child { 0 } else { job_level };
                if let Some(careers) = program.careers.as_ref()
                    && let Some(career) = careers.iter().find(|c| c.id == i32::from(job_type))
                    && let Ok(lvl) = usize::try_from(level_idx)
                    && let Some(level) = career.levels.get(lvl)
                {
                    let live_female = (gender_word & 1) != 0;
                    // 0x45ef51, 0x45ef63, 0x45ef75: CARR workwear templates and
                    // secondary suit are copied into 64-byte (`63`-byte) buffers
                    // initialized at `0x4593c8`–`0x45940e`.
                    let tmpl_raw = if live_female && !level.strings[2].is_empty() {
                        &level.strings[2]
                    } else {
                        &level.strings[1]
                    };
                    let tmpl = truncate_cp1252(tmpl_raw, 63);
                    let tex_tmpl = truncate_cp1252(&level.strings[3], 63);
                    init_clothing_rows(program, world, guid, profile);
                    body_suit = match_registered_suit(
                        program,
                        world,
                        tmpl,
                        gender_code,
                        &body_type,
                        &skin_3,
                    );
                    body_tex = match_registered_texture(
                        program,
                        tex_tmpl,
                        gender_code,
                        &body_type,
                        &skin_3,
                        &body_row_tex,
                        "BODY=",
                        63,
                    );
                    body_alt_suit = truncate_cp1252(&level.strings[4], 63).to_owned();
                }
                outfit.workwear_cache = Some(WorkwearCache {
                    job_type,
                    job_level,
                    body_suit: body_suit.clone(),
                    body_tex: body_tex.clone(),
                    body_alt_suit: body_alt_suit.clone(),
                });
            }
        }
        4 => {
            // 0x58a6ff..0x58a7b0: formalwear checks `atoi(STR# 200 row 13) <= 17`,
            // then checks raw row 30 presence BEFORE parsing (`0x58a733`).
            if is_formal_child {
                (body_suit, body_tex) = row_desc(27);
            } else if raw_row(30).is_some_and(|r| !r.is_empty()) {
                (body_suit, body_tex) = row_desc(30);
            } else {
                (body_suit, body_tex) = row_desc(27);
                (extra_left_suit, extra_left_tex) = row_desc(28);
                (extra_right_suit, extra_right_tex) = row_desc(29);
            }
        }
        5 => {
            // 0x58a7f0: checks raw row 32 presence BEFORE parsing; otherwise
            // matched pajamas (`0x45f034`).
            if raw_row(32).is_some_and(|r| !r.is_empty()) {
                (body_suit, body_tex) = row_desc(32);
            } else {
                let tmpl = if is_word_child {
                    "pjs$gchd"
                } else {
                    "pajama$g$b"
                };
                init_clothing_rows(program, world, guid, profile);
                body_suit =
                    match_registered_suit(program, world, tmpl, gender_code, &body_type, &skin_3);
                body_tex = match_registered_texture(
                    program,
                    "pajama$g$c",
                    gender_code,
                    &body_type,
                    &skin_3,
                    &body_row_tex,
                    "BODY=",
                    63,
                );
            }
        }
        6 | 7 => {
            // 0x45f15a: skeleton (suit 6) and electric-shock skeleton (suit 7).
            let shock = suit == 7;
            if is_word_child {
                body_suit = "skeletonchd_01".to_owned();
                head_suit = "c_kidskull".to_owned();
            } else {
                body_suit = "skeleton_01".to_owned();
                head_suit = "c_skeleton".to_owned();
            }
            if shock {
                body_tex = "BODY=skeleneg_01".to_owned();
                head_tex = "HEAD-HEAD=c_skeleneg".to_owned();
                extra_left_tex = "HAND=_GUAO_sneg".to_owned();
                extra_right_tex = "HAND=_GUAO_sneg".to_owned();
            } else {
                body_tex = "BODY=skeleton_01".to_owned();
                head_tex = "HEAD-HEAD=c_skeleton".to_owned();
                extra_left_tex = "HAND=_GUAO_skel".to_owned();
                extra_right_tex = "HAND=_GUAO_skel".to_owned();
            }
        }
        8..=16 => {
            // 0x45f452: themed party costumes (`ct-{prefix}-$gC-{v:02}` /
            // `ct-{prefix}-$gAFit-{v:02}` with `v = data[21] + 1`; texture
            // inserts `$c` before `-{v:02}`).
            const PREFIXES: [&str; 9] = [
                "toga", "cow", "luau", "rave", "cost", "nice", "swim", "pj", "disco",
            ];
            let prefix = PREFIXES[(suit - 8) as usize];
            let v = i32::from(variant) + 1;
            let (suit_tmpl, tex_tmpl) = if is_word_child {
                (
                    format!("ct-{prefix}-$gC-{v:02}"),
                    format!("ct-{prefix}-$gC$c-{v:02}"),
                )
            } else {
                (
                    format!("ct-{prefix}-$gAFit-{v:02}"),
                    format!("ct-{prefix}-$gAFit$c-{v:02}"),
                )
            };
            init_clothing_rows(program, world, guid, profile);
            body_suit =
                match_registered_suit(program, world, &suit_tmpl, gender_code, &body_type, &skin_3);
            body_tex = match_registered_texture(
                program,
                &tex_tmpl,
                gender_code,
                &body_type,
                &skin_3,
                &body_row_tex,
                "BODY=",
                63,
            );
        }
        17 => {
            (body_suit, body_tex) = row_desc(33);
        }
        18 => {
            (body_suit, body_tex) = row_desc(34);
        }
        19..=60 => {
            let mut skip_hand_templates = false;
            if matches!(suit, 55 | 56) {
                // 0x58c3cf, 0x58c3dc: mummy suits clear live words 22 and 23.
                if let Some(person) = world
                    .objects
                    .get_mut(&person_id)
                    .and_then(|o| o.person.as_mut())
                {
                    if let Some(w) = person.data.get_mut(HAND_LEFT) {
                        *w = 0;
                    }
                    if let Some(w) = person.data.get_mut(HAND_RIGHT) {
                        *w = 0;
                    }
                }
                world.routing.unknown_words.remove(&(person_id, HAND_LEFT));
                world.routing.unknown_words.remove(&(person_id, HAND_RIGHT));
                // 0x58c5d9, 0x58c5f0: variant 3 binds `nohandl` and `nohandr`
                // BEFORE `0x409315` / `0x40e5ce` / `0x40e9b2` at `0x58c76f`.
                if variant == 3 {
                    extra_left_suit = "nohandl".to_owned();
                    extra_left_tex = "nohand".to_owned();
                    extra_right_suit = "nohandr".to_owned();
                    extra_right_tex = "nohand".to_owned();
                    if slot_differs(&outfit.extra_left, &extra_left_suit, &extra_left_tex) {
                        bind_slot(
                            &program.people,
                            world,
                            &mut outfit.extra_left,
                            &extra_left_suit,
                            &extra_left_tex,
                        );
                    }
                    if slot_differs(&outfit.extra_right, &extra_right_suit, &extra_right_tex) {
                        bind_slot(
                            &program.people,
                            world,
                            &mut outfit.extra_right,
                            &extra_right_suit,
                            &extra_right_tex,
                        );
                    }
                    skip_hand_templates = true;
                }
            }
            if let Some(spec) = costume_spec(suit, variant) {
                init_clothing_rows(program, world, guid, profile);
                if let Some(b) = spec.body {
                    body_suit =
                        match_registered_suit(program, world, b, gender_code, &body_type, &skin_3);
                }
                if let Some(bt) = spec.body_tex {
                    body_tex = match_registered_texture(
                        program,
                        bt,
                        gender_code,
                        &body_type,
                        &skin_3,
                        &body_row_tex,
                        "BODY=",
                        63,
                    );
                }
                if let Some(h) = spec.head {
                    head_suit =
                        match_registered_suit(program, world, h, gender_code, &body_type, &skin_3);
                }
                if let Some(ht) = spec.head_tex {
                    // 0x58c7c9 passes the 256-byte (`255`-byte) destination
                    // initialized at `0x58a3d5`–`0x58a3eb`.
                    head_tex = match_registered_texture(
                        program,
                        ht,
                        gender_code,
                        &body_type,
                        &skin_3,
                        &head_row_tex,
                        "HEAD-HEAD=",
                        255,
                    );
                }
                if !skip_hand_templates {
                    if let Some(hl) = spec.hand_left {
                        extra_left_tex = expand_hand_template(hl, hand_left_word, gender_code);
                    }
                    if let Some(hr) = spec.hand_right {
                        extra_right_tex = expand_hand_template(hr, hand_right_word, gender_code);
                    }
                }
            }
        }
    }

    // 0x58d494..0x58d4bc: when `body_suit` is empty and `suit != 0`, fall back
    // to everyday body row 1.
    if body_suit.is_empty() && suit != 0 {
        (body_suit, body_tex) = row_desc(1);
    }

    // 0x58d4c1..0x58d51d: inspect the EXISTING head slot (`[ebx + 0x258]`)
    // BEFORE rebinding:
    // - empty head slot -> `0x58d4e1 -> 0x4eb0cc -> 0x4e897e` clears cached
    //   portraits AND clears dirty flags, and schedules head rebind;
    // - bound head slot whose `(suit, texture)` differs (`0x58a12f`) ->
    //   `0x58d518 -> 0x4eb0eb -> 0x4e895e` clears cached portraits AND marks
    //   dirty, and schedules head rebind;
    // - unchanged bound head slot -> skips `0x58d606` and leaves portrait state alone.
    let rebind_head = if outfit.head.is_none() {
        world.portraits.remove(&guid);
        world.dirty_portraits.remove(&guid);
        true
    } else if slot_differs(&outfit.head, &head_suit, &head_tex) {
        world.portraits.remove(&guid);
        world.dirty_portraits.insert(guid);
        true
    } else {
        false
    };

    // 0x58d51e..0x58d5a3: check whether body (`[esi][0]`) and body_alt
    // (`[esi][1]`) need rebinding.
    let rebind_body = slot_differs(&outfit.body, &body_suit, &body_tex);
    let rebind_body_alt = if outfit.body_alt.is_some() {
        slot_differs(&outfit.body_alt, &body_alt_suit, "")
    } else {
        !body_alt_suit.is_empty()
    };

    // 0x58d5b4..0x58d70f: bind head, body, and body_alt in exact native order.
    if rebind_head {
        bind_slot(
            &program.people,
            world,
            &mut outfit.head,
            &head_suit,
            &head_tex,
        );
    }
    if rebind_body {
        bind_slot(
            &program.people,
            world,
            &mut outfit.body,
            &body_suit,
            &body_tex,
        );
    }
    if rebind_body_alt {
        bind_slot(
            &program.people,
            world,
            &mut outfit.body_alt,
            &body_alt_suit,
            "",
        );
    }

    // 0x58d712..0x58d75e: bind extra_left (`0x58d742`) and extra_right
    // (`0x58d759`) via `0x58a070` (which checks `0x58a12f`) whenever BOTH have
    // non-empty suit names.
    if !extra_left_suit.is_empty() && !extra_right_suit.is_empty() {
        if slot_differs(&outfit.extra_left, &extra_left_suit, &extra_left_tex) {
            bind_slot(
                &program.people,
                world,
                &mut outfit.extra_left,
                &extra_left_suit,
                &extra_left_tex,
            );
        }
        if slot_differs(&outfit.extra_right, &extra_right_suit, &extra_right_tex) {
            bind_slot(
                &program.people,
                world,
                &mut outfit.extra_right,
                &extra_right_suit,
                &extra_right_tex,
            );
        }
    }

    world.person_outfits.insert(person_id, outfit);
    Ok(())
}
