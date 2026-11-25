//! `.antics` v2 typed `std` foundation: `std::flags`, `std::engine`, typed engine fields, casts,
//! and the expanded `std::globals`. All sources are synthetic.

use sims_antics::{PackResolverConfig, compile_v2_source};

fn expand(name: &str, src: &str) -> String {
    let cfg = PackResolverConfig::default();
    compile_v2_source(name, src, &cfg).unwrap_or_else(|e| panic!("{name} must compile: {e}"));
    sims_antics::v2::expand_v2_source_to_objects(name, src, &cfg)
        .unwrap_or_else(|e| panic!("{name} must expand: {e}"))
        .into_iter()
        .next()
        .expect("one object")
        .air_source
}

fn iff(name: &str, src: &str) -> Vec<u8> {
    compile_v2_source(name, src, &PackResolverConfig::default())
        .unwrap_or_else(|e| panic!("{name} must compile: {e}"))
        .iff_bytes
}

fn expect_err(name: &str, src: &str, needle: &str) {
    let cfg = PackResolverConfig::default();
    let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
    assert!(
        err.message.contains(needle),
        "{name}: expected '{needle}' in: {err}"
    );
}

fn expect_lines(name: &str, air: &str, lines: &[&str]) {
    for want in lines {
        assert!(air.contains(want), "{name}: missing `{want}` in:\n{air}");
    }
}

/// A synthetic object with `uses` before it and one Sim tree `t` running `body`.
fn sim_tree(uses: &str, body: &str) -> String {
    format!(
        r#"{uses}
object "Typed Std" {{
    guid: 0x7F40_0250;
    state n: i16;
    #[bhav] fn t(sim: Sim) {{ {body} }}
}}
"#
    )
}

#[test]
fn std_flags_and_engine_constants_are_their_engine_numbers() {
    let air = expand(
        "consts.antics",
        &sim_tree(
            "use std::flags::*;\nuse std::engine::{Direction, PersonKind};",
            "self.n = ObjectFlags::Hidden as i16; \
             self.n = (ObjectFlags::InUse | ObjectFlags::NotSolid) as i16; \
             self.n = Direction::SouthWest as i16; \
             self.n = PersonKind::FemaleCat as i16; \
             self.n = Expansions::Magic as i16;",
        ),
    );
    expect_lines(
        "consts",
        &air,
        &[
            "stack.attr[0] := 16384;",
            "stack.attr[0] := 36;",
            "stack.attr[0] := 5;",
            "stack.attr[0] := 128;",
        ],
    );
}

#[test]
fn from_raw_and_as_i16_cost_nothing() {
    let air = expand(
        "casts.antics",
        &sim_tree(
            "use std::flags::WallPlacement;\nuse std::engine::Direction;",
            "let d: Direction = Direction::from_raw(self.n); \
             self.n = d as i16; \
             let w = WallPlacement::from_raw(self.n); \
             if w flag_test WallPlacement::NoCorner { self.n = 1; }",
        ),
    );
    expect_lines(
        "casts",
        &air,
        &[
            // `d` folds into its source word; the cast adds no node.
            "stack.attr[0] := stack.attr[0];",
            "let __v_w_1 = stack.attr[0];",
            "if __v_w_1 flag_test 5 {",
        ],
    );
    expect_err(
        "cast_u8.antics",
        &sim_tree("", "self.n = self.n as u8;"),
        "expected 'i16' after 'as'",
    );
    expect_err(
        "cast_handle.antics",
        &sim_tree("", "self.n = stack_obj as i16;"),
        "as i16",
    );
    expect_err(
        "from_raw_two.antics",
        &sim_tree(
            "use std::engine::Direction;",
            "let d = Direction::from_raw(1, 2);",
        ),
        "'Direction::from_raw' takes exactly one positional i16 argument",
    );
}

#[test]
fn height_and_placement_stay_prelude_types() {
    let body = "self.allowed_heights = Height::Floor | Height::Table; \
                self.placement = Placement::Floor | Placement::Pool;";
    let bare = iff("bare.antics", &sim_tree("", body));
    let imported = iff(
        "imported.antics",
        &sim_tree("use std::flags::{Height, Placement};", body),
    );
    let globbed = iff("globbed.antics", &sim_tree("use std::flags::*;", body));
    assert_eq!(bare, imported);
    assert_eq!(bare, globbed);
    expect_err(
        "redeclare.antics",
        "bitflags Height { A = 1 }\nobject \"P\" { guid: 0x7F40_0250; }\n",
        "Height",
    );
}

#[test]
fn a_match_on_an_engine_enum_needs_a_wildcard() {
    let arms = |last: &str| {
        sim_tree(
            "use std::engine::Direction;",
            &format!(
                "match self.facing {{ Direction::North => {{ self.n = 1; }} {last} => {{ self.n = 2; }} }}"
            ),
        )
    };
    expect_err(
        "no_wildcard.antics",
        &arms("Direction::East"),
        "a match on engine enum 'Direction' needs a '_' arm",
    );
    let air = expand("wildcard.antics", &arms("_"));
    expect_lines("wildcard", &air, &["if stack.data[1] == 0 {", "} else {"]);
    // A user enum stays closed: every variant covers the match.
    let user = r#"enum Two { A = 0, B = 1 }
object "P" {
    guid: 0x7F40_0250;
    state n: i16;
    #[bhav] fn t(sim: Sim) {
        let e: Two = Two::A;
        match e { Two::A => { self.n = 1; } Two::B => { self.n = 2; } }
    }
}
"#;
    expand("user_enum.antics", user);
}

#[test]
fn typed_engine_fields_lower_to_their_words() {
    let air = expand(
        "fields.antics",
        &sim_tree(
            "use std::flags::{ObjectFlags2, BuyFlags, WallPlacement};\nuse std::engine::{Direction, PersonKind};",
            "self.facing = Direction::South; \
             self.n = self.weight + self.support_strength; \
             self.n = self.value + self.category; \
             if self.flags2 flag_test ObjectFlags2::LightSource { self.n = 1; } \
             self.buy_flags flag_clear BuyFlags::CanSell; \
             self.wall_placement flag_set WallPlacement::NeedsCorner; \
             stack_obj = self.container; \
             match sim.gender_kind { PersonKind::Female => { self.n = 2; } _ => {} } \
             sim.date_partner = 0; \
             self.n = sim.facing as i16; \
             stack_obj.value = 10;",
        ),
    );
    expect_lines(
        "fields",
        &air,
        &[
            "stack.data[1] := 4;",
            // The tree moves the cursor, so `self` arithmetic is staged (PR I-4).
            "let __arith_0 = stack.data[27];",
            "__arith_0 += stack.data[28];",
            "__arith_0 := stack.data[41];",
            "__arith_0 += stack.data[59];",
            "if stack.data[40] flag_test 11 {",
            "stack.data[43] flag_clear 4;",
            "stack.data[13] flag_set 6;",
            "stack_obj := stack.data[2];",
            "if my.person[65] == 1 {",
            "my.person[77] := 0;",
            "stack.attr[0] := my.data[1];",
            "stack.data[41] := 10;",
        ],
    );
}

#[test]
fn engine_field_types_are_enforced() {
    expect_err(
        "container_write.antics",
        &sim_tree("", "self.container = stack_obj;"),
        "cannot assign to 'self.container': it is read-only",
    );
    expect_err(
        "handle_container_write.antics",
        &sim_tree("", "stack_obj.container = stack_obj;"),
        "cannot assign to 'stack_obj.container': it is read-only",
    );
    expect_err(
        "facing_int.antics",
        &sim_tree("", "self.facing = 3;"),
        "cannot assign 'i16' expression to 'Direction' target",
    );
    expect_err(
        "facing_read.antics",
        &sim_tree("", "self.n = self.facing;"),
        "cannot assign 'Direction' expression to 'i16' target",
    );
    // A state of the same name wins over the engine word.
    let air = expand(
        "state_wins.antics",
        r#"object "P" {
    guid: 0x7F40_0250;
    state value: i16;
    state container: i16;
    #[bhav] fn t(sim: Sim) { self.value = 3; self.container = 4; }
}
"#,
    );
    expect_lines(
        "state_wins",
        &air,
        &["stack.attr[0] := 3;", "stack.attr[1] := 4;"],
    );
}

#[test]
fn engine_flag_words_keep_their_numeric_spellings() {
    let air = expand(
        "numeric.antics",
        &sim_tree(
            "",
            "self.wall_placement flag_set 3; \
             self.wall_placement = 5; \
             self.n = self.wall_placement; \
             self.n = self.wall_placement & 4; \
             if self.wall_placement flag_test 2 { self.n = 1; }",
        ),
    );
    expect_lines(
        "numeric",
        &air,
        &[
            "stack.data[13] flag_set 3;",
            "stack.data[13] := 5;",
            "stack.attr[0] := stack.data[13];",
            "if stack.data[13] flag_test 2 {",
        ],
    );
}

#[test]
fn an_i16_word_takes_a_typed_single_flag() {
    let air = expand(
        "i16_flags.antics",
        &sim_tree(
            "use std::flags::{Expansions, WallPlacement};",
            "if world.expansions flag_test Expansions::Pets { self.n = 1; } \
             global[20] flag_set Expansions::Magic; \
             self.n flag_clear WallPlacement::NoCorner;",
        ),
    );
    expect_lines(
        "i16_flags",
        &air,
        &[
            "if global[20] flag_test 6 {",
            "global[20] flag_set 8;",
            "stack.attr[0] flag_clear 5;",
        ],
    );
    expect_err(
        "i16_mask.antics",
        &sim_tree(
            "use std::flags::Expansions;",
            "self.n flag_set Expansions::Pets | Expansions::Magic;",
        ),
        "requires a single flag constant",
    );
    // A user bitflags type stays strict: no raw bit numbers.
    expect_err(
        "user_strict.antics",
        r#"bitflags Mine { A = 1, B = 2 }
object "P" {
    guid: 0x7F40_0250;
    state n: i16;
    state m: Mine;
    #[bhav] fn t(sim: Sim) { self.m flag_set 2; }
}
"#,
        "requires a single flag constant",
    );
}

#[test]
fn primitives_take_engine_constants_as_their_numbers() {
    let typed = iff(
        "prims_typed.antics",
        &sim_tree(
            "use std::engine::*;",
            "refresh(target: RefreshTarget::StackObject, mode: RefreshMode::RecountRoom); \
             if can_afford(100, category: BudgetCategory::Build) { self.n = 1; } \
             debit_funds(5, category: BudgetCategory::Buy);",
        ),
    );
    let numbers = iff(
        "prims_numbers.antics",
        &sim_tree(
            "",
            "refresh(target: 1, mode: 2); \
             if can_afford(100, category: 7) { self.n = 1; } \
             debit_funds(5, category: 6);",
        ),
    );
    assert_eq!(typed, numbers);
}

#[test]
fn named_words_build_the_same_nodes_as_their_numbers() {
    let named = iff(
        "named.antics",
        &sim_tree(
            "use std::engine::Direction;",
            "stack_obj = sim.id; self.n = 1; \
             temp[0] = sim.room; temp[1] = self.room; \
             self.facing = Direction::West; \
             temp[2] = self.value; \
             stack_obj = self.id;",
        ),
    );
    let numbered = iff(
        "numbered.antics",
        &sim_tree(
            "",
            "stack_obj = sim.data[11]; self.n = 1; \
             temp[0] = sim.data[29]; temp[1] = self.data[29]; \
             self.data[1] = 6; \
             temp[2] = self.data[41]; \
             stack_obj = self.data[11];",
        ),
    );
    assert_eq!(named, numbered);
    let lifecycle = |body: &str| {
        format!(
            r#"object "P" {{
    guid: 0x7F40_0250;
    state n: i16;
    #[bhav] fn t() {{ {body} }}
}}
"#
        )
    };
    assert_eq!(
        iff(
            "self_named.antics",
            &lifecycle("stack_obj = self.id; temp[0] = self.room;")
        ),
        iff(
            "self_numbered.antics",
            &lifecycle("stack_obj = self.data[11]; temp[0] = self.data[29];")
        ),
    );
}

#[test]
fn std_globals_call_the_documented_trees() {
    let air = expand(
        "globals.antics",
        &sim_tree(
            "use std::globals::*;",
            "if !globals::ensure_standing() { return false; } \
             globals::change_to_swimwear(1)?; \
             if globals::find_companion() { self.n = temp[0]; } \
             globals::rouse_stack_room(0, 1)?; \
             if globals::wants_fun(50) { self.n = 1; } \
             globals::wait_hours(2)?;",
        ),
    );
    expect_lines(
        "globals",
        &air,
        &[
            "if !call 382(temps) {",
            "if !call 377(1) {",
            "if call 333(temps) {",
            "if !call 399(0, 1) {",
            "if call 285(50) {",
            "if !call 366(2) {",
        ],
    );
}

#[test]
fn flag_operations_never_take_a_mask_as_a_bit() {
    expect_err(
        "runtime_mask.antics",
        &sim_tree("", "let f = Height::from_raw(8); temp[0] flag_set f;"),
        "takes a single flag constant or an i16 bit number, found 'Height'",
    );
    expect_err(
        "runtime_mask_engine.antics",
        &sim_tree("", "let f = Height::from_raw(8); self.flags flag_clear f;"),
        "takes a single flag constant or an i16 bit number",
    );
    let two = "bitflags First { One = 5 }\nbitflags Second { Two = 5 }";
    expect_err(
        "mixed_set.antics",
        &sim_tree(two, "temp[0] flag_set First::One | Second::Two;"),
        "type mismatch",
    );
    expect_err(
        "mixed_test.antics",
        &sim_tree(
            two,
            "if temp[0] flag_test First::One | Second::Two { temp[1] = 1; }",
        ),
        "type mismatch",
    );
}

#[test]
fn from_raw_keeps_user_enums_closed() {
    expect_err(
        "closed.antics",
        &sim_tree(
            "enum Mode { Idle = 0, Active = 1 }",
            "let m = Mode::from_raw(99);",
        ),
        "'Mode::from_raw' is only for 'std::engine' enums and bitflags types",
    );
    // A user bitflags type converts: any mask is a value of it.
    expand(
        "user_flags.antics",
        &sim_tree(
            "bitflags Mine { A = 1 }",
            "let m = Mine::from_raw(self.n); self.n = m as i16;",
        ),
    );
}

#[test]
fn engine_words_compare_and_convert_like_i16() {
    let air = expand(
        "compat.antics",
        &sim_tree(
            "use std::engine::Direction;",
            "if self.wall_placement > 0 { temp[0] = 1; } \
             random_into(self.wall_placement, 6); \
             if self.flags2 > 0 { temp[0] = 2; } \
             if temp[0] == Direction::East as i16 { temp[1] = 7; } \
             if self.facing == Direction::from_raw(2) { temp[1] = 8; } \
             refresh(target: true as i16, mode: 0);",
        ),
    );
    expect_lines(
        "compat",
        &air,
        &[
            "if stack.data[13] > 0 {",
            "random(stack.data[13], 6);",
            "if stack.data[40] > 0 {",
            "if temp[0] == 2 {",
            "if stack.data[1] == 2 {",
            "refresh(target: 1, mode: 0);",
        ],
    );
    assert!(!air.contains("__cmp_l"), "a cast must not stage: {air}");
    let casts = " as i16".repeat(65);
    expect_err(
        "deep_casts.antics",
        &sim_tree("", &format!("temp[0] = 1{casts};")),
        "nesting depth exceeds maximum",
    );
    // `show_skill_progress` keeps its parameter name for named calls.
    let air = expand(
        "skill_named.antics",
        &sim_tree(
            "use std::globals::*;",
            "if !globals::show_skill_progress(skill: 250) { return error; }",
        ),
    );
    expect_lines("skill_named", &air, &["call 286(250)"]);
}

#[test]
fn engine_enum_parameters_get_no_verifier_domain() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("v2-typed-std-domains-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let source = dir.join("open.antics");
    std::fs::write(
        &source,
        r#"use std::engine::Direction;
enum Closed { A = 0, B = 1 }
object "Domains" {
    guid: 0x7F40_0250;
    state n: i16;
    #[bhav] fn open(sim: Sim, d: Direction) {
        match d { Direction::North => { self.n = 1; } _ => {} }
    }
    #[bhav] fn closed(sim: Sim, c: Closed) {
        match c { Closed::A => { self.n = 1; } Closed::B => {} }
    }
}
"#,
    )
    .expect("write source");
    let found = sims_antics::verify::v2_enum_param_domains(&source, None).expect("extract");
    let names: Vec<&str> = found.iter().map(|d| d.param_name.as_str()).collect();
    // A `std::engine` word may hold any value, so only the closed user enum is assumed.
    assert_eq!(names, ["c"], "{found:#?}");
}

#[test]
fn casts_and_raw_conversions_stay_sound() {
    // A declared callable of the same path wins over the conversion.
    let air = expand(
        "shadowed.antics",
        &sim_tree(
            "extern mod Height { #[id(280)] #[ret(temp = 0)] fn from_raw(i: i16) -> i16; }",
            "if Height::from_raw(1) == 1 { temp[0] = 5; }",
        ),
    );
    expect_lines("shadowed", &air, &["call 280(1)"]);
    expect_err(
        "bool_short_circuit.antics",
        &sim_tree("", "refresh(target: (true || missing) as i16, mode: 0);"),
        "must be a compile-time constant",
    );
    expect_err(
        "runtime_bool.antics",
        &sim_tree("", "temp[0] = stack_obj.is_visitor as i16;"),
        "'as i16' converts only a constant bool",
    );
    expect_err(
        "unrelated_order.antics",
        &sim_tree(
            "use std::flags::*;",
            "if self.flags2 < self.buy_flags { temp[0] = 1; }",
        ),
        "ordering comparison is not supported between 'ObjectFlags2' and 'BuyFlags'",
    );
    let many = " as i16".repeat(40);
    let air = expand(
        "collapsed.antics",
        &sim_tree("", &format!("temp[0] = (1{many}){many};")),
    );
    expect_lines("collapsed", &air, &["temp[0] := 1;"]);
    let air = expand(
        "const_casts.antics",
        &sim_tree(
            "use std::engine::Direction;",
            "drop_onto(src: -32768 as i16, dst: 2); \
             refresh(target: Direction::from_raw(1) as i16, mode: 0); \
             temp[0] flag_set sim.id;",
        ),
    );
    expect_lines(
        "const_casts",
        &air,
        &[
            "drop_onto(src: -32768, dst: 2);",
            "refresh(target: 1, mode: 0);",
            "temp[0] flag_set my.id;",
        ],
    );
    // 382 keeps its `-> Result` signature, so a local declaration of it still merges.
    expand(
        "merge_382.antics",
        &sim_tree(
            "use std::globals::*;\nextern mod globals { #[id(382)] fn get_up() -> Result; }",
            "globals::get_up()?;",
        ),
    );
}

#[test]
fn constant_casts_follow_the_value_rules() {
    // An `extern mod` named like the type owns `T::from_raw` in every position.
    for (name, body) in [
        (
            "owned_cond.antics",
            "if Height::from_raw(1) == Height::Floor { temp[0] = 9; }",
        ),
        ("owned_let.antics", "let h = Height::from_raw(1);"),
    ] {
        expect_err(
            name,
            &sim_tree(
                "extern mod Height { #[id(280)] fn other() -> Result; }",
                body,
            ),
            "extern mod 'Height' has no fn 'from_raw'",
        );
    }
    expect_err(
        "closed_index.antics",
        &sim_tree(
            "enum Closed { Only = 0 }",
            "temp[Closed::from_raw(1) as i16] = 7;",
        ),
        "'Closed' is a closed enum",
    );
    expect_err(
        "wide_operand.antics",
        &sim_tree("", "generic_call(routine: 32768 as i16);"),
        "integer 32768 out of i16 range",
    );
    expect_err(
        "runtime_operand.antics",
        &sim_tree("", "refresh(target: temp[0] as i16, mode: 0);"),
        "refresh target must be a compile-time constant",
    );
    expect_err(
        "foreign_raw.antics",
        &sim_tree(
            "use std::engine::*;",
            "refresh(target: Direction::from_raw(RefreshMode::RecountRoom) as i16, mode: 0);",
        ),
        "'Direction::from_raw' expects an i16, found 'RefreshMode'",
    );
    let source = r#"use std::engine::Direction;
object "Typed Std" {
    guid: 0x7F40_0250;
    state n: i16;
    param enabled: bool = true;
    #[bhav] fn t(sim: Sim) {
        refresh(target: Direction::from_raw(Direction::NorthEast) as i16, mode: 0);
        drop_onto(src: Direction::from_raw(-32768) as i16, dst: 2);
        temp[0] = (!false) as i16;
        refresh(target: (!false) as i16, mode: 0);
        temp[1] = enabled as i16;
        refresh(target: enabled as i16, mode: 0);
        self.n = (temp[0] + temp[1]) as i16;
    }
}
"#;
    let air = expand("const_operands.antics", source);
    expect_lines(
        "const_operands",
        &air,
        &[
            "refresh(target: 1, mode: 0);",
            "drop_onto(src: -32768, dst: 2);",
            "temp[0] := 1;",
            "temp[1] := 1;",
            "stack.attr[0] := temp[0];",
            "stack.attr[0] += temp[1];",
        ],
    );
    // The cast costs no node: the same bytes as the uncast sum.
    let sum = |cast: &str| {
        iff(
            "sum.antics",
            &sim_tree("", &format!("self.n = (temp[0] + temp[1]){cast};")),
        )
    };
    assert_eq!(sum(" as i16"), sum(""));
}
