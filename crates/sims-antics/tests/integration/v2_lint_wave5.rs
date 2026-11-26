//! `.antics` v2 lint, wave 5: `collapsible_else_if`. An `else { .. }` that holds one `if`
//! (with its own `else if` / `else` chain) and nothing else is `else if ..`, as Clippy's
//! `collapsible_else_if` says for Rust. Both spellings parse to the same tree, so each
//! positive case here also builds both and compares the bytes.
//!
//! Every fixture here is synthetic (GUIDs in the clean-room 0x7E00_xxxx range). See
//! `docs/decisions/20261125-antics-v2-lint-collapsible-else-if.md`.

use sims_antics::v2::PackResolverConfig;
use sims_antics::v2::lint::{LintDiagnostic, Severity, lint_v2_source};

const RULE: &str = "collapsible_else_if";

/// A synthetic object-context tree running `body`, with an `i16` argument `a`.
fn tree(body: &str) -> String {
    format!(
        "object \"Synth Probe\" {{\n    guid: 0x7E00_0500;\n    price: 1;\n    #[bhav(label = \"Probe\")] fn probe(a: i16) {{\n{body}\n    }}\n}}\n"
    )
}

fn lint(source: &str) -> Vec<LintDiagnostic> {
    lint_v2_source(source, "probe.antics").unwrap_or_else(|e| panic!("lint failed: {e}\n{source}"))
}

fn of(source: &str, rule: &str) -> Vec<LintDiagnostic> {
    lint(source)
        .into_iter()
        .filter(|d| d.rule == rule)
        .collect()
}

fn none(source: &str, rule: &str) {
    let found = of(source, rule);
    assert!(
        found.is_empty(),
        "unexpected {rule} in\n{source}\n{found:#?}"
    );
}

fn iff(source: &str) -> Vec<u8> {
    sims_antics::compile_v2_source("probe.antics", source, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("must compile: {e}\n{source}"))
        .iff_bytes
}

/// `body` gets a `collapsible_else_if` at each `(line, column)` in `at` (1-based, in
/// `body`), and nothing else the rule reports; `collapsed` builds the same bytes and gets
/// none.
fn collapses(body: &str, at: &[(usize, usize)], collapsed: &str) {
    let source = tree(body);
    let found = of(&source, RULE);
    // `tree` puts four lines before the body.
    let lines: Vec<(usize, usize)> = found.iter().map(|d| (d.line - 4, d.column)).collect();
    assert_eq!(lines, at, "{source}\n{found:#?}");
    for d in &found {
        assert_eq!(d.severity, Severity::Warning, "{d:#?}");
        assert!(
            d.help.as_deref().is_some_and(|h| h.contains("else if")),
            "{d:#?}"
        );
    }
    let fixed = tree(collapsed);
    none(&fixed, RULE);
    assert_eq!(iff(&source), iff(&fixed), "{source}\nvs\n{fixed}");
}

// --- reported ------------------------------------------------------------------------------

#[test]
fn an_else_holding_only_an_if_is_reported() {
    collapses(
        "        if a == 1 {
            temp[0] = 1;
        } else {
            if a == 2 {
                temp[0] = 2;
            }
        }",
        &[(3, 11)],
        "        if a == 1 {
            temp[0] = 1;
        } else if a == 2 {
            temp[0] = 2;
        }",
    );
}

#[test]
fn the_inner_if_may_carry_its_own_else_chain() {
    collapses(
        "        if a == 1 {
            temp[0] = 1;
        } else {
            if a == 2 {
                temp[0] = 2;
            } else if a == 3 {
                temp[0] = 3;
            } else {
                temp[0] = 4;
            }
        }",
        &[(3, 11)],
        "        if a == 1 {
            temp[0] = 1;
        } else if a == 2 {
            temp[0] = 2;
        } else if a == 3 {
            temp[0] = 3;
        } else {
            temp[0] = 4;
        }",
    );
}

#[test]
fn the_last_else_of_an_else_if_chain_is_reported() {
    collapses(
        "        if a == 1 {
            temp[0] = 1;
        } else if a == 2 {
            temp[0] = 2;
        } else {
            if a == 3 {
                temp[0] = 3;
            }
        }",
        &[(5, 11)],
        "        if a == 1 {
            temp[0] = 1;
        } else if a == 2 {
            temp[0] = 2;
        } else if a == 3 {
            temp[0] = 3;
        }",
    );
}

#[test]
fn nested_else_blocks_are_each_reported() {
    collapses(
        "        if a == 1 {
            temp[0] = 1;
        } else {
            if a == 2 {
                temp[0] = 2;
            } else {
                if a == 3 {
                    temp[0] = 3;
                }
            }
        }",
        &[(3, 11), (6, 15)],
        "        if a == 1 {
            temp[0] = 1;
        } else if a == 2 {
            temp[0] = 2;
        } else if a == 3 {
            temp[0] = 3;
        }",
    );
}

#[test]
fn an_if_nested_in_a_then_branch_is_reported() {
    collapses(
        "        if a > 0 {
            if a == 1 {
                temp[0] = 1;
            } else {
                if a == 2 {
                    temp[0] = 2;
                }
            }
        }",
        &[(4, 15)],
        "        if a > 0 {
            if a == 1 {
                temp[0] = 1;
            } else if a == 2 {
                temp[0] = 2;
            }
        }",
    );
}

#[test]
fn an_if_expression_else_is_reported() {
    collapses(
        "        let x = if a == 1 { 1 } else { if a == 2 { 2 } else { 3 } };
        temp[0] = x;",
        &[(1, 33)],
        "        let x = if a == 1 { 1 } else if a == 2 { 2 } else { 3 };
        temp[0] = x;",
    );
}

#[test]
fn one_line_blocks_are_reported() {
    collapses(
        "        if a == 1 { temp[0] = 1; } else { if a == 2 { temp[0] = 2; } }",
        &[(1, 36)],
        "        if a == 1 { temp[0] = 1; } else if a == 2 { temp[0] = 2; }",
    );
}

#[test]
fn comments_the_collapse_keeps_do_not_block_it() {
    // Inside the inner `if`'s blocks, and before the `else`: the collapse removes only the
    // `else` block's own braces, so these comments stay.
    collapses(
        "        if a == 1 {
            temp[0] = 1;
        } // one
        else {
            if a == 2 {
                // two
                temp[0] = 2;
            } else {
                temp[0] = 3; // three
            }
        }",
        &[(4, 9)],
        "        if a == 1 {
            temp[0] = 1;
        } // one
        else if a == 2 {
            // two
            temp[0] = 2;
        } else {
            temp[0] = 3; // three
        }",
    );
}

// --- not reported -----------------------------------------------------------------------

#[test]
fn else_if_is_already_collapsed() {
    none(
        &tree(
            "        if a == 1 {
            temp[0] = 1;
        } else if a == 2 {
            temp[0] = 2;
        } else {
            temp[0] = 3;
        }",
        ),
        RULE,
    );
    none(
        &tree(
            "        let x = if a == 1 { 1 } else if a == 2 { 2 } else { 3 };\n        temp[0] = x;",
        ),
        RULE,
    );
}

#[test]
fn an_else_with_more_than_the_if_is_not_reported() {
    for body in [
        "            if a == 2 {\n                temp[0] = 2;\n            }\n            temp[1] = 1;",
        "            temp[1] = 1;\n            if a == 2 {\n                temp[0] = 2;\n            }",
        "            let b = a + 1;\n            if b == 2 {\n                temp[0] = b;\n            }",
    ] {
        none(
            &tree(&format!(
                "        if a == 1 {{\n            temp[0] = 1;\n        }} else {{\n{body}\n        }}"
            )),
            RULE,
        );
    }
}

#[test]
fn a_comment_the_collapse_would_drop_blocks_it() {
    for (before, between, after) in [
        // Between `else` and `{`.
        (" /* zero */ {", "\n            ", "\n        }"),
        // Between `{` and `if`, on the `else` line or a line of its own.
        (" { // zero", "\n            ", "\n        }"),
        (" {", "\n            // zero\n            ", "\n        }"),
        (" {", " /* zero */ ", "\n        }"),
        // After the inner `if`, before the `else` block's `}`.
        (" {", "\n            ", " // zero\n        }"),
        (" {", "\n            ", "\n            // zero\n        }"),
        (" {", "\n            ", " /* zero */ }"),
    ] {
        let source = tree(&format!(
            "        if a == 1 {{\n            temp[0] = 1;\n        }} else{before}{between}if a == 2 {{\n                temp[0] = 2;\n            }}{after}"
        ));
        none(&source, RULE);
    }
    // In an `if` expression too.
    none(
        &tree(
            "        let x = if a == 1 { 1 } else { /* zero */ if a == 2 { 2 } else { 3 } };\n        temp[0] = x;",
        ),
        RULE,
    );
}

#[test]
fn raw_blocks_are_not_read() {
    // `raw { }` is opaque `.air`: an `else` holding one is no `if`, and `.air` text that
    // looks like v2 is never parsed as v2.
    none(
        &tree(
            "        if a == 1 {
            temp[0] = 1;
        } else {
            raw { temp[0] := 2; # else { if a == 2 { } }
            }
        }",
        ),
        RULE,
    );
}

#[test]
fn other_else_blocks_are_not_if_else() {
    // A `for` loop's `else` takes a block only: `else if` does not parse there.
    none(
        &tree(
            "        for o in lot.objects() {
            temp[0] = 1;
        } else {
            if a == 2 {
                temp[0] = 2;
            }
        }",
        ),
        RULE,
    );
}

#[test]
fn an_inner_condition_with_braces_is_left_alone() {
    // The token walk does not read braces inside a condition; such an `if` is skipped
    // (a false negative), never misread.
    none(
        &tree(
            "        if a == 1 {
            temp[0] = 1;
        } else {
            if (if a == 2 { true } else { false }) {
                temp[0] = 2;
            }
        }",
        ),
        RULE,
    );
}

// --- suppression and the open pack ------------------------------------------------------

#[test]
fn the_rule_can_be_allowed() {
    let source = tree(
        "        if a == 1 {
            temp[0] = 1;
        // antics-lint: allow(collapsible_else_if)
        } else {
            if a == 2 {
                temp[0] = 2;
            }
        }",
    );
    none(&source, RULE);
    none(&source, "stale_lint_allow");
}

#[test]
fn the_open_pack_has_no_collapsible_else_if() {
    let pack = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../open-pack");
    let (diags, _) = sims_antics::v2::lint::lint_v2_paths(&[pack]).expect("lint");
    assert!(diags.is_empty(), "{diags:#?}");
}
