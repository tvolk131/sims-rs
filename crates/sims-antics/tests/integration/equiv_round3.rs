//! Verifier round 3 (Track 7): local-slot renaming inside cut-point (bisimulation)
//! proofs, and why a declared enum's domain is never assumed without
//! `--v2-enum-domains`. Each rule has a case it proves and adversarial cases it must
//! not. Every tree and string here is synthetic.

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::{
    EquivResult, IffEquivOptions, LiveContext, LiveOptions, ResourceVerdict,
    check_bhav_live_equivalent, check_iff_equivalent,
};
use sims_formats::behavior::Instruction;
use sims_formats::iff::{ChunkData, write as write_iff};

const T: u8 = 0xFE;
const F: u8 = 0xFF;

// Expression scopes and operators.
const LIT: u8 = 7;
const PARAM: u8 = 9;
const LOCAL: u8 = 25;
const ATTR: u8 = 0;
const EQ: u8 = 2;
const ADD: u8 = 3;
const SET: u8 = 5;

fn expr(lhs: (u8, i16), op: u8, rhs: (u8, i16), on_true: u8, on_false: u8) -> Instruction {
    let mut operands = [0u8; 8];
    operands[0..2].copy_from_slice(&lhs.1.to_le_bytes());
    operands[2..4].copy_from_slice(&rhs.1.to_le_bytes());
    operands[5] = op;
    operands[6] = lhs.0;
    operands[7] = rhs.0;
    Instruction {
        opcode: 2,
        on_true,
        on_false,
        operands,
    }
}

fn prim(opcode: u16, operands: [u8; 8], on_true: u8, on_false: u8) -> Instruction {
    Instruction {
        opcode,
        on_true,
        on_false,
        operands,
    }
}

fn tree(id: u16, args: u8, locals: u16, instructions: Vec<Instruction>) -> BhavChunk {
    BhavChunk {
        id,
        label: format!("Tree{id}"),
        format: 0x8002,
        kind: 0,
        args,
        locals,
        revision: 0,
        instructions,
        trailing: Vec::new(),
    }
}

fn live_with(a: &BhavChunk, b: &BhavChunk, opts: &LiveOptions) -> EquivResult {
    let ctx = LiveContext::default();
    check_bhav_live_equivalent(a, &ctx, b, &ctx, opts)
}

fn behavioral() -> LiveOptions {
    LiveOptions {
        behavioral: true,
        ..LiveOptions::default()
    }
}

#[track_caller]
fn assert_equivalent(result: &EquivResult) {
    assert!(
        matches!(result, EquivResult::Equivalent { .. }),
        "expected Equivalent, got {result:?}"
    );
}

#[track_caller]
fn assert_not_equivalent(result: &EquivResult) {
    assert!(
        !matches!(result, EquivResult::Equivalent { .. }),
        "expected a non-Equivalent verdict, got {result:?}"
    );
}

fn bhav_chunk(bhav: &BhavChunk) -> ChunkData {
    ChunkData {
        tag: *b"BHAV",
        id: bhav.id,
        flags: 0x10,
        label: bhav.label.clone(),
        data: encode_bhav(bhav).expect("encode_bhav"),
    }
}

/// A version -3 string table of US English strings, read by index (flag 0x10).
fn strings(id: u16, texts: &[&str]) -> ChunkData {
    let mut data = Vec::new();
    data.extend_from_slice(&(-3i16).to_le_bytes());
    data.extend_from_slice(&u16::try_from(texts.len()).unwrap().to_le_bytes());
    for text in texts {
        data.push(1);
        data.extend_from_slice(text.as_bytes());
        data.extend_from_slice(b"\0\0");
    }
    ChunkData {
        tag: *b"STR#",
        id,
        flags: 0x10,
        label: String::new(),
        data,
    }
}

fn file(chunks: Vec<ChunkData>) -> Vec<u8> {
    write_iff(&chunks).expect("write")
}

fn verdict_of(report: &sims_antics::IffEquivReport, id: u16) -> &ResourceVerdict {
    &report
        .resources
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no resource #{id}: {report:#?}"))
        .verdict
}

fn check(a: &[u8], b: &[u8], live: LiveOptions) -> sims_antics::IffEquivReport {
    let opts = IffEquivOptions {
        live,
        ..IffEquivOptions::default()
    };
    check_iff_equivalent(a, b, &opts).expect("check")
}

// ----------------------------------------------------------------------------
// Local-slot renaming inside cut-point proofs
// ----------------------------------------------------------------------------

/// Alter Budget (25) by `local[slot]`.
fn budget_from(slot: i16) -> [u8; 8] {
    let mut ops = [2, 0, 0, 0, 0, 0, 0, 0];
    ops[2..4].copy_from_slice(&slot.to_le_bytes());
    ops
}

/// A loop with no bound the checker can see: count `local[counter]` from 0 up to
/// `param[0]`, altering the budget by `local[read]` each time round. Path enumeration
/// cannot finish it, so it is proved (or not) by the cut-point bisimulation.
fn counting_loop(counter: i16, read: i16) -> BhavChunk {
    tree(
        4096,
        1,
        4,
        vec![
            expr((LOCAL, counter), SET, (LIT, 0), 1, 1),
            expr((LOCAL, counter), EQ, (PARAM, 0), T, 2),
            prim(25, budget_from(read), 3, F),
            expr((LOCAL, counter), ADD, (LIT, 1), 1, 1),
        ],
    )
}

/// The loop is beyond path enumeration: `param[0]` may be any of 32768 non-negative
/// values, and reaching each takes three nodes per iteration, far past the step budget
/// every path must fit in. So the same tree against itself can only be proved by the
/// cut-point bisimulation, and it is.
#[test]
fn the_counting_loop_needs_the_cut_point_proof() {
    let opts = behavioral();
    assert!(
        opts.max_steps < 3 * 32767,
        "the loop must not fit the path budget"
    );
    let a = counting_loop(0, 0);
    assert_equivalent(&live_with(&a, &a, &opts));
}

/// A port that keeps the loop counter in another slot is the same behavior: the
/// renaming applies before the cut-point proof as well as before path enumeration.
/// Strict mode observes frame storage and never renames.
#[test]
fn a_loop_counter_in_another_slot_is_renamed_inside_a_cut_point_proof() {
    for slot in [1, 2, 3] {
        let (a, b) = (counting_loop(0, 0), counting_loop(slot, slot));
        assert_equivalent(&live_with(&a, &b, &behavioral()));
        assert_not_equivalent(&live_with(&a, &b, &LiveOptions::default()));
    }
}

/// Adversarial: B reads a slot it never writes (its budget amount is whatever an
/// earlier frame left there), so no renaming may equate it with A's counter.
#[test]
fn a_loop_reading_an_unwritten_slot_is_not_renamed() {
    let a = counting_loop(0, 0);
    for (counter, read) in [(1, 0), (1, 2), (0, 1)] {
        let b = counting_loop(counter, read);
        assert_not_equivalent(&live_with(&a, &b, &behavioral()));
    }
}

/// Adversarial: two of B's locals hold different values where A's one local is read.
/// B counts in `local[1]` but reads `local[2]`, which it sets once to 0 and never
/// changes: the first iteration agrees, the second does not.
#[test]
fn a_loop_reading_a_stale_copy_is_not_renamed() {
    let a = counting_loop(0, 0);
    let b = tree(
        4096,
        1,
        4,
        vec![
            expr((LOCAL, 1), SET, (LIT, 0), 1, 1),
            expr((LOCAL, 2), SET, (LIT, 0), 2, 2),
            expr((LOCAL, 1), EQ, (PARAM, 0), T, 3),
            prim(25, budget_from(2), 4, F),
            expr((LOCAL, 1), ADD, (LIT, 1), 2, 2),
        ],
    );
    assert_not_equivalent(&live_with(&a, &b, &behavioral()));
}

/// A Dialog (36) in the loop body showing message `message` of the table.
fn dialog_loop(counter: i16, message: u8) -> BhavChunk {
    tree(
        4096,
        1,
        4,
        vec![
            expr((LOCAL, counter), SET, (LIT, 0), 1, 1),
            expr((LOCAL, counter), EQ, (PARAM, 0), T, 2),
            prim(36, [0, 0, message, 0, 0, 0, 0, 0], 3, 3),
            expr((LOCAL, counter), ADD, (LIT, 1), 1, 1),
        ],
    )
}

fn dialog_loop_files(counter_b: i16, texts: &[&str]) -> (Vec<u8>, Vec<u8>) {
    let side = |counter| {
        file(vec![
            bhav_chunk(&dialog_loop(counter, 1)),
            strings(301, texts),
        ])
    };
    (side(0), side(counter_b))
}

/// The dialog's string names no local, so the counter may move; a string naming the
/// counter's slot (`$Local:0`) pins it, inside a cut-point proof as before.
#[test]
fn dialog_text_pins_a_loop_counter_inside_a_cut_point_proof() {
    let (a, b) = dialog_loop_files(2, &["Plain words."]);
    let report = check(&a, &b, behavioral());
    assert!(
        matches!(
            verdict_of(&report, 4096),
            ResourceVerdict::Equivalent { .. }
        ),
        "{report:#?}"
    );
    for texts in [["Count $Local:0."], ["Hello $Me."], ["Count $Local:2."]] {
        let (a, b) = dialog_loop_files(2, &texts);
        let report = check(&a, &b, behavioral());
        assert!(
            !matches!(
                verdict_of(&report, 4096),
                ResourceVerdict::Equivalent { .. } | ResourceVerdict::Identical
            ),
            "{texts:?}: {report:#?}"
        );
    }
}

// ----------------------------------------------------------------------------
// Enum domains are never assumed by default
// ----------------------------------------------------------------------------

/// A `match` over a declared enum `{0, 1, 2}` with no `_` arm, as the compiler lowers
/// it: the original falls through to a fault past the last arm (Breakpoint, 15, on the
/// literal 1, which always faults), and the port ends the last arm with no test at all.
fn exhaustive_match(port: bool) -> BhavChunk {
    let mut nodes = vec![
        expr((PARAM, 0), EQ, (LIT, 0), 3, 1),
        expr((PARAM, 0), EQ, (LIT, 1), 4, 2),
    ];
    if port {
        nodes.push(expr((ATTR, 0), SET, (LIT, 30), T, T));
    } else {
        nodes.push(expr((PARAM, 0), EQ, (LIT, 2), 5, 6));
    }
    nodes.push(expr((ATTR, 0), SET, (LIT, 10), T, T));
    nodes.push(expr((ATTR, 0), SET, (LIT, 20), T, T));
    if !port {
        nodes.push(expr((ATTR, 0), SET, (LIT, 30), T, T));
        nodes.push(prim(15, [1, 0, LIT, 0, 0, 0, 0, 0], T, T));
    }
    let mut chunk = tree(4097, 1, 0, nodes);
    chunk.label = "Match".to_owned();
    chunk
}

/// A caller of the same file passing only literals in the enum's domain.
fn literal_caller(id: u16, value: i16) -> BhavChunk {
    let mut ops = [0u8; 8];
    ops[0..2].copy_from_slice(&value.to_le_bytes());
    tree(id, 0, 0, vec![prim(4097, ops, T, F)])
}

fn match_file(port: bool) -> Vec<u8> {
    file(vec![
        bhav_chunk(&exhaustive_match(port)),
        bhav_chunk(&literal_caller(4098, 0)),
        bhav_chunk(&literal_caller(4099, 2)),
    ])
}

/// Every caller in the file passes a literal in the enum's domain, but the tree is
/// still not proved without an assumption: Run Tree By Name (28) lets any object run
/// it by its label with its own temps as the arguments (`named_tree.rs`), and a queued
/// interaction's arguments carry an object id. An out-of-domain value really reaches
/// the fault on one side only, so the verdict is never EQUIVALENT.
#[test]
fn an_exhaustive_match_is_not_proved_by_its_callers_alone() {
    let (a, b) = (match_file(false), match_file(true));
    for live in [behavioral(), LiveOptions::default()] {
        let report = check(&a, &b, live);
        assert!(
            !matches!(
                verdict_of(&report, 4097),
                ResourceVerdict::Equivalent { .. } | ResourceVerdict::Identical
            ),
            "{report:#?}"
        );
    }
    // The tree pair on its own: an out-of-domain value takes A to the fault only. (The
    // replay runs only trees made of Expressions, so this stays INCONCLUSIVE rather
    // than a replayed counterexample; either way it is not EQUIVALENT.)
    let result = live_with(
        &exhaustive_match(false),
        &exhaustive_match(true),
        &behavioral(),
    );
    assert_not_equivalent(&result);
}

/// With the domain given (what `--v2-enum-domains` supplies, as an ASSUME line), the
/// same pair is proved; a domain that admits one more value is not.
#[test]
fn an_exhaustive_match_is_proved_only_under_an_assumed_domain() {
    let (a, b) = (exhaustive_match(false), exhaustive_match(true));
    let ctx = |set: &str| {
        let mut domains = sims_antics::verify::ParamDomains::new();
        domains.entry(4097).or_default().insert(
            0,
            sims_antics::verify::ParamDomain::parse(set).expect("domain"),
        );
        LiveContext {
            param_domains: domains,
            ..LiveContext::default()
        }
    };
    let opts = behavioral();
    let inside = ctx("0..=2");
    assert_equivalent(&check_bhav_live_equivalent(&a, &inside, &b, &inside, &opts));
    for set in ["0..=3", "-1..=2", "0,1,2,7"] {
        let wider = ctx(set);
        assert_not_equivalent(&check_bhav_live_equivalent(&a, &wider, &b, &wider, &opts));
    }
}
