//! `pad` clauses: operand bytes the VM ignores, spelled on the named primitive.
//!
//! Every tree here is synthetic. A `pad` must rebuild the exact bytes, and only bytes
//! `canonicalize_primitive_operands` says the VM reads no differently may be padded.

use sims_antics::encode::encode_bhav;
use sims_antics::ir::BhavChunk;
use sims_antics::{
    EquivOptions, EquivResult, LiftOptions, StructuredLiftOutcome, check_bhav_equivalent,
    compile_air, compile_antics, format_bhav_labeled_air, lift_bhav_structured_with_options,
};
use sims_formats::behavior::Instruction;

const T: u8 = 0xFE;
const F: u8 = 0xFF;

fn tree(instructions: Vec<Instruction>) -> BhavChunk {
    BhavChunk {
        id: 4096,
        label: "Padded".to_owned(),
        format: 0x8002,
        kind: 0,
        args: 1,
        locals: 1,
        revision: 0,
        instructions,
        trailing: Vec::new(),
    }
}

fn ins(opcode: u16, operands: [u8; 8], on_true: u8, on_false: u8) -> Instruction {
    Instruction {
        opcode,
        on_true,
        on_false,
        operands,
    }
}

/// The operands one `.air` line compiles to.
fn operands_of(line: &str) -> [u8; 8] {
    let src = format!(
        "tree 4096 \"Padded\" (format: 0x8002, kind: 0, args: 1, locals: 1, revision: 0) {{\n    @n0:\n    {line} -> true: true, false: false;\n}}\n"
    );
    compile_air(&src)
        .unwrap_or_else(|e| panic!("{line}: {e}"))
        .behaviors[&4096]
        .instructions[0]
        .operands
}

fn compile_error(line: &str) -> String {
    let src = format!(
        "tree 4096 \"Padded\" (format: 0x8002, kind: 0, args: 1, locals: 1, revision: 0) {{\n    @n0:\n    {line} -> true: true, false: false;\n}}\n"
    );
    match compile_air(&src) {
        Ok(_) => panic!("{line}: expected a compile error"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn pad_rebuilds_the_exact_bytes() {
    // Operands ignored entirely.
    assert_eq!(operands_of("grab() pad 0xff"), [0xFF; 8]);
    assert_eq!(operands_of("notify_idle() pad 0xff"), [0xFF; 8]);
    // Expression: only byte 4 is never read.
    assert_eq!(
        operands_of("expr(temp[0] := 1) pad 0xff"),
        [0, 0, 1, 0, 0xFF, 5, 8, 7]
    );
    // Breakpoint: bytes 4..8 are never read.
    assert_eq!(
        operands_of("breakpoint(1) pad 0xff"),
        [1, 0, 7, 0, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    // A scope word above 255 is refused before any read (native error 9).
    assert_eq!(
        operands_of("breakpoint(invalid_scope: 0xffff, index: -1) pad 0xff"),
        [0xFF; 8]
    );
    // Alter Budget: selector 3 with an explicit scope byte reads like the short
    // selector; bytes 5 and 7 and the upper six flag bits are never read.
    assert_eq!(
        operands_of("alter_budget(local[0], category: 3, flags: 1) pad(0: 0x03, 7: 0x02)"),
        [3, 25, 0, 0, 1, 0, 3, 2]
    );
    assert_eq!(
        operands_of("alter_budget(param[0], category: 0, flags: 1) pad(1: 0x00)"),
        [1, 0, 0, 0, 1, 0, 0, 0]
    );
    assert_eq!(
        operands_of("alter_budget(param[0], category: 0, flags: 1) pad(4: 0xfd)"),
        [1, 9, 0, 0, 0xFD, 0, 0, 0]
    );
}

#[test]
fn pad_refuses_bytes_the_vm_reads() {
    // Refresh reads words 0 and 1.
    let e = compile_error("refresh(target: 0, mode: 1) pad(2: 0x05)");
    assert!(e.contains("reads"), "{e}");
    // Flag bit 0 of Alter Budget is read.
    let e = compile_error("alter_budget(param[0], category: 0, flags: 1) pad(4: 0xfe)");
    assert!(e.contains("reads"), "{e}");
    // Expression operator and scopes are read.
    let e = compile_error("expr(temp[0] := 1) pad(5: 0x03)");
    assert!(e.contains("reads"), "{e}");
    // With selector 3 the scope byte is read.
    let e = compile_error("alter_budget(temp[0], category: 0, flags: 0) pad(1: 0x09)");
    assert!(e.contains("reads"), "{e}");
    // Random Number reads every byte, so nothing can be padded.
    let e = compile_error("random(temp[0], 5) pad 0xff");
    assert!(e.contains("changes no operand byte"), "{e}");
    let e = compile_error("random(temp[0], 5) pad(7: 0x01)");
    assert!(e.contains("reads"), "{e}");
}

#[test]
fn pad_rejects_no_ops_and_malformed_clauses() {
    let e = compile_error("grab() pad 0x00");
    assert!(e.contains("changes no operand byte"), "{e}");
    let e = compile_error("grab() pad(3: 0x00)");
    assert!(e.contains("already holds"), "{e}");
    let e = compile_error("grab() pad(5: 0x01, 4: 0x01)");
    assert!(e.contains("ascending"), "{e}");
    let e = compile_error("grab() pad(4: 0x01, 4: 0x02)");
    assert!(e.contains("ascending"), "{e}");
    let e = compile_error("grab() pad(8: 0x01)");
    assert!(e.contains("0..=7"), "{e}");
    let e = compile_error("breakpoint(invalid_scope: 7, index: 0)");
    assert!(e.contains("valid scope"), "{e}");
}

/// Strict lifting spells padded instructions with `pad` instead of `op 0xNNNN (...)`,
/// byte for byte in labeled form and equivalently in structured form.
#[test]
fn strict_lift_spells_padding() {
    let bhav = tree(vec![
        ins(4, [0xFF; 8], 1, F),
        ins(2, [0, 0, 1, 0, 0xFF, 5, 8, 7], 2, F),
        ins(15, [0xFF; 8], 3, F),
        ins(15, [1, 0, 7, 0, 0xFF, 0xFF, 0xFF, 0xFF], 4, F),
        ins(25, [3, 25, 0, 0, 1, 0, 3, 2], 5, F),
        ins(25, [1, 0, 0, 0, 1, 0, 0, 0], 6, F),
        ins(2, [0, 0, 1, 0, 0xFE, 2, 8, 7], 7, F),
        ins(49, [0xFF; 8], T, F),
    ]);
    let labeled = format_bhav_labeled_air(&bhav, LiftOptions::default());
    for want in [
        "grab() pad 0xff",
        "expr(temp[0] := 1) pad 0xff",
        "breakpoint(invalid_scope: 0xffff, index: -1) pad 0xff",
        "breakpoint(1) pad 0xff",
        "alter_budget(local[0], category: 3, flags: 1) pad(0: 0x03, 7: 0x02)",
        // Bytes 1, 5 and 7 are the free ones here, and all hold 0.
        "alter_budget(param[0], category: 0, flags: 1) pad 0x00",
        "expr(temp[0] == 1) pad 0xfe",
        "notify_idle() pad 0xff",
    ] {
        assert!(labeled.contains(want), "missing {want}:\n{labeled}");
    }
    assert!(!labeled.contains("op 0x"), "{labeled}");
    let rebuilt = &compile_air(&labeled).expect("labeled compiles").behaviors[&4096];
    assert_eq!(
        encode_bhav(rebuilt).unwrap(),
        encode_bhav(&bhav).unwrap(),
        "{labeled}"
    );

    let StructuredLiftOutcome::Lifted(structured) =
        lift_bhav_structured_with_options(&bhav, LiftOptions::default())
    else {
        panic!("expected a structured lift");
    };
    assert!(!structured.contains("op 0x"), "{structured}");
    // A padded comparison is a condition.
    assert!(
        structured.contains("if !expr(temp[0] == 1) pad 0xfe"),
        "{structured}"
    );
    let rebuilt = &compile_antics(&structured)
        .expect("structured compiles")
        .behaviors[&4096];
    assert!(
        matches!(
            check_bhav_equivalent(&bhav, rebuilt, &EquivOptions::default()),
            EquivResult::Equivalent { .. }
        ),
        "{structured}"
    );
}

/// Bytes that the VM reads stay raw `op` instructions: padding never stands in for
/// a different read.
#[test]
fn strict_lift_keeps_read_bytes_raw() {
    // An Expression operator `.air` does not name (21): the padded byte alone would
    // be spellable, but the operator is read, so the whole instruction stays raw.
    let bhav = tree(vec![ins(2, [0, 0, 1, 0, 0xFF, 21, 8, 7], T, F)]);
    let labeled = format_bhav_labeled_air(&bhav, LiftOptions::default());
    assert!(labeled.contains("op 0x0002"), "{labeled}");
    let rebuilt = &compile_air(&labeled).expect("labeled compiles").behaviors[&4096];
    assert_eq!(encode_bhav(rebuilt).unwrap(), encode_bhav(&bhav).unwrap());
}

/// Raw `op 0xNNNN (...)` stays accepted for every opcode, padded or not.
#[test]
fn raw_op_spelling_still_compiles() {
    for (op, bytes) in [
        ("0x0004", [0xFFu8; 8]),
        ("0x000f", [0xFF; 8]),
        ("0x0019", [3, 25, 0, 0, 1, 0, 3, 2]),
        ("0x002c", [1, 0, 0, 0, 3, 0x20, 0, 0]),
        ("0x0031", [0xFF; 8]),
        ("0x0002", [0, 0, 1, 0, 0xFF, 5, 8, 7]),
    ] {
        let list: Vec<String> = bytes.iter().map(|b| format!("{b:#04x}")).collect();
        let line = format!("op {op} ({})", list.join(", "));
        assert_eq!(operands_of(&line), bytes, "{line}");
    }
}

/// Dialog byte 5 bit 7 is copied into the dialog record the VM keeps, so neither
/// lifting mode drops it and `pad` cannot set it.
#[test]
fn dialog_kind_bit_7_stays_raw() {
    let bhav = tree(vec![ins(36, [0, 0, 0, 0, 0, 0x85, 0, 1], T, F)]);
    for strict_padding in [true, false] {
        let opts = LiftOptions {
            strict_padding,
            annotate_comments: false,
        };
        let labeled = format_bhav_labeled_air(&bhav, opts);
        assert!(labeled.contains("op 0x0024"), "{labeled}");
        let rebuilt = &compile_air(&labeled).expect("labeled compiles").behaviors[&4096];
        assert_eq!(encode_bhav(rebuilt).unwrap(), encode_bhav(&bhav).unwrap());
    }
}
